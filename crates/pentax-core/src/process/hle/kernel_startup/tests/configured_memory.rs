// SPDX-License-Identifier: GPL-2.0-only
//! Actual owned backing, current rights, split pool records and atomic outputs.
use super::*;

fn read16(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64) -> [u8; 16] {
    let mut bytes = [0; 16];
    session.pause().unwrap().read(address, &mut bytes).unwrap();
    bytes
}

#[test]
fn configured_size_actual_cross_page_outputs_recheck_current_rights_and_preserve_whole_span() {
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
            let output = mapped + PAGE_SIZE - 3;
            session
                .pause()
                .unwrap()
                .write(output - 4, &[0xa5; 16])
                .unwrap();
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::ConfiguredFlexibleMemorySize,
                    [0, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]
                )),
                Ok(0x8002_0016)
            );
            assert_eq!(read16(&mut session, output - 4), [0xa5; 16]);
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::ConfiguredFlexibleMemorySize,
                    [output, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]
                )),
                Ok(0)
            );
            let mut expected = [0xa5; 16];
            expected[4..12].copy_from_slice(&(64_u64 * 1024 * 1024).to_le_bytes());
            assert_eq!(read16(&mut session, output - 4), expected);
            session
                .pause()
                .unwrap()
                .write(output - 4, &[0xa5; 16])
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
                    Service::ConfiguredFlexibleMemorySize,
                    [output, 0, 0, 0, 0, 0]
                )),
                Err(KernelStartupError::Kernel(Failure::Guest(_)))
            ));
            assert_eq!(read16(&mut session, output - 4), [0xa5; 16]);
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
                    Service::ConfiguredFlexibleMemorySize,
                    [output, 0, 0, 0, 0, 0]
                )),
                Ok(0)
            );
            assert_eq!(read16(&mut session, output - 4), expected);
            let tail = mapped + 2 * PAGE_SIZE - 16;
            session.pause().unwrap().write(tail, &[0x5a; 16]).unwrap();
            for address in [tail + 13, tail + 16, u64::MAX - 3] {
                assert!(matches!(
                    session.kernel_stop(stop(
                        plan,
                        Service::ConfiguredFlexibleMemorySize,
                        [address, 0, 0, 0, 0, 0]
                    )),
                    Err(KernelStartupError::Kernel(Failure::Guest(_)))
                ));
                assert_eq!(read16(&mut session, tail), [0x5a; 16]);
            }
            let before = session.kernel_usage();
            let mut forged = stop(
                plan,
                Service::ConfiguredFlexibleMemorySize,
                [output, 0, 0, 0, 0, 0],
            );
            forged.syscall_pc += 1;
            assert!(matches!(
                session.kernel_stop(forged),
                Err(KernelStartupError::SiteMismatch { .. })
            ));
            assert_eq!(session.kernel_usage().attempts, before.attempts);
            assert_eq!(read16(&mut session, output - 4), expected);
        },
    );
}

#[test]
fn configured_pool_counts_protection_splits_and_partial_unmaps_but_not_imported_image_pages() {
    let mut limits = KernelStartupLimits::default();
    limits.kernel.flexible_memory_bytes = 3 * PAGE_SIZE;
    with_plan(limits, |plan, owner, base, pointers| {
        let mut session = prepared(plan, owner, &pointers);
        // This adopts a real loaded image page; it shares aggregate accounting
        // while remaining outside the dedicated flexible pool.
        session
            .kernel_stop(stop(
                plan,
                Service::Mprotect,
                [base + 0x4000, PAGE_SIZE, 3, 0, 0, 0],
            ))
            .unwrap();
        session
            .kernel_stop(stop(
                plan,
                Service::MapFlexible,
                [base + OUTPUT, 2 * PAGE_SIZE, 3, 0, 0, 0],
            ))
            .unwrap();
        let first = word(&mut session, base + OUTPUT);
        session
            .kernel_stop(stop(
                plan,
                Service::Mprotect,
                [first + PAGE_SIZE, PAGE_SIZE, 1, 0, 0, 0],
            ))
            .unwrap();
        session
            .pause()
            .unwrap()
            .write(base + OUTPUT + 8, &0_u64.to_le_bytes())
            .unwrap();
        let resources = session.kernel_usage();
        assert_eq!(
            session.kernel_stop(stop(
                plan,
                Service::MapFlexible,
                [base + OUTPUT + 8, 2 * PAGE_SIZE, 3, 0, 0, 0]
            )),
            Ok(0x8002_000c)
        );
        assert_eq!(word(&mut session, base + OUTPUT + 8), 0);
        assert_eq!(
            (
                session.kernel_usage().mapped_bytes,
                session.kernel_usage().handles_live
            ),
            (resources.mapped_bytes, resources.handles_live)
        );
        session
            .kernel_stop(stop(plan, Service::Munmap, [first, PAGE_SIZE, 0, 0, 0, 0]))
            .unwrap();
        assert_eq!(
            session.kernel_stop(stop(
                plan,
                Service::MapFlexible,
                [base + OUTPUT + 8, 2 * PAGE_SIZE, 3, 0, 0, 0]
            )),
            Ok(0)
        );
        let replacement = word(&mut session, base + OUTPUT + 8);
        let mut zeros = [1; 16];
        session
            .pause()
            .unwrap()
            .read(replacement, &mut zeros)
            .unwrap();
        assert_eq!(zeros, [0; 16]);
        assert_eq!(
            session.kernel_stop(stop(
                plan,
                Service::ConfiguredFlexibleMemorySize,
                [base + OUTPUT + 32, 0, 0, 0, 0, 0]
            )),
            Ok(0)
        );
        assert_eq!(word(&mut session, base + OUTPUT + 32), 3 * PAGE_SIZE);
        session
            .pause()
            .unwrap()
            .write(base + OUTPUT + 16, &0_u64.to_le_bytes())
            .unwrap();
        assert_eq!(
            session.kernel_stop(stop(
                plan,
                Service::MapFlexible,
                [base + OUTPUT + 16, PAGE_SIZE, 3, 0, 0, 0]
            )),
            Ok(0x8002_000c)
        );
        assert_eq!(word(&mut session, base + OUTPUT + 16), 0);
        session
            .kernel_stop(stop(
                plan,
                Service::Munmap,
                [first + PAGE_SIZE, PAGE_SIZE, 0, 0, 0, 0],
            ))
            .unwrap();
        assert_eq!(
            session.kernel_stop(stop(
                plan,
                Service::MapFlexible,
                [base + OUTPUT + 16, PAGE_SIZE, 3, 0, 0, 0]
            )),
            Ok(0)
        );
        assert_eq!(
            session.kernel_stop(stop(
                plan,
                Service::ConfiguredFlexibleMemorySize,
                [base + OUTPUT + 32, 0, 0, 0, 0, 0]
            )),
            Ok(0)
        );
        assert_eq!(word(&mut session, base + OUTPUT + 32), 3 * PAGE_SIZE);
    });
}
