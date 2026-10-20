// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn errno_cell(owner: &mut Kernel, guest: &mut TestGuest) -> u64 {
    call(owner, guest, Service::ErrnoLocation, &[]).unwrap()
}

fn count(owner: &mut Kernel, guest: &mut TestGuest, address: u64) -> u32 {
    guest.put(32, &[0xa5; 8]);
    assert_eq!(
        call(
            owner,
            guest,
            Service::PthreadSemGetValue,
            &[address, ARG + 32]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 36, 4), [0xa5; 4]);
    u32::from_le_bytes(guest.get(ARG + 32, 4).try_into().unwrap())
}

#[test]
fn pthread_semaphore_layout_counts_reinitialization_and_destroy_match_source() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = errno_cell(&mut owner, &mut guest);
    guest.copy(errno, &99_u32.to_le_bytes());
    let baseline = owner.usage().handles_live;
    guest.put(0, &[0xa5; 24]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemInit,
            &[ARG, 0xdead_beef, 2]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG, 16),
        [
            0xfa, 9, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0xef, 0xbe, 0xad, 0xde
        ]
    );
    assert_eq!(guest.get(ARG + 16, 8), [0xa5; 8]);
    assert_eq!(owner.usage().handles_live, baseline + 1);
    // Success rewrites only the first12 bytes, preserving guest-visible flags.
    guest.put(12, &0x1234_5678_u32.to_le_bytes());
    for service in [Service::PosixSemWait, Service::PthreadSemTryWait] {
        assert_eq!(call(&mut owner, &mut guest, service, &[ARG]), Ok(0));
    }
    assert_eq!(count(&mut owner, &mut guest, ARG), 0);
    assert_eq!(guest.get(ARG + 12, 4), 0x1234_5678_u32.to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemPost, &[ARG]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemPost, &[ARG]),
        Ok(0)
    );
    assert_eq!(count(&mut owner, &mut guest, ARG), 2);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PthreadSemInit,
            &[ARG, 0, 7, 0]
        ),
        Ok(0)
    );
    assert_eq!(count(&mut owner, &mut guest, ARG), 7);
    assert_eq!(owner.usage().handles_live, baseline + 1);
    guest.put(2, &[0x47, 0x89]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemDestroy, &[ARG]),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG, 16),
        [0, 0, 0x47, 0x89, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(owner.usage().handles_live, baseline);
    assert_eq!(guest.get(errno, 4), 99_u32.to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemPost, &[ARG]),
        Err(Errno::Invalid.into())
    );
}

#[test]
fn pthread_semaphore_faults_keep_counts_outputs_and_lifetime_unchanged() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 24]);
    for address in [ARG + PAGE - 8, u64::MAX - 8] {
        assert!(matches!(
            call(
                &mut owner,
                &mut guest,
                Service::PthreadSemInit,
                &[address, 0, 1]
            ),
            Err(Failure::Guest(_))
        ));
        assert_eq!(owner.usage().handles_live, 0);
    }
    call(
        &mut owner,
        &mut guest,
        Service::PthreadSemInit,
        &[ARG, 0, 1],
    )
    .unwrap();
    let before = guest.get(ARG, 24);
    for service in [
        Service::PthreadSemWait,
        Service::PthreadSemPost,
        Service::PthreadSemDestroy,
        Service::PthreadSemInit,
    ] {
        guest.refuse = true;
        assert!(matches!(
            call(&mut owner, &mut guest, service, &[ARG, 0, 9]),
            Err(Failure::Guest(_))
        ));
        guest.refuse = false;
        assert_eq!(guest.get(ARG, 24), before);
        assert_eq!(count(&mut owner, &mut guest, ARG), 1);
        assert_eq!(owner.usage().handles_live, 1);
    }
    call(&mut owner, &mut guest, Service::PthreadSemWait, &[ARG]).unwrap();
    assert_eq!(count(&mut owner, &mut guest, ARG), 0);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemWait, &[ARG]),
        Err(Failure::Unsupported("POSIX semaphore blocking wait"))
    );
    call(&mut owner, &mut guest, Service::PthreadSemDestroy, &[ARG]).unwrap();
    assert_eq!(owner.usage().handles_live, 0);
}

#[test]
fn pthread_semaphore_posix_errno_and_prefix_are_one_transaction() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = errno_cell(&mut owner, &mut guest);
    call(&mut owner, &mut guest, Service::PosixSemInit, &[ARG, 0, 0]).unwrap();
    guest.put(0, &[0x55; 16]);
    guest.copy(errno, &7_u32.to_le_bytes());
    let region = guest
        .regions
        .iter()
        .position(|region| region.start <= errno && errno < region.start + region.length)
        .unwrap();
    guest.regions[region].protection = 1;
    assert!(matches!(
        call(&mut owner, &mut guest, Service::PosixSemTryWait, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG, 16), [0x55; 16]);
    assert_eq!(guest.get(errno, 4), 7_u32.to_le_bytes());
    guest.regions[region].protection = 3;
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemTryWait, &[ARG]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(guest.get(errno, 4), 35_u32.to_le_bytes());
    assert_eq!(guest.get(ARG, 12), [0xfa, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(guest.get(ARG + 12, 4), [0x55; 4]);
    call(
        &mut owner,
        &mut guest,
        Service::PosixSemInit,
        &[errno, 0, 0],
    )
    .unwrap();
    let before = guest.get(errno, 16);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemTryWait, &[errno]),
        Err(Failure::Unsupported("overlapping guest outputs"))
    );
    assert_eq!(guest.get(errno, 16), before);
}

#[test]
fn pthread_semaphore_timeouts_and_both_error_domains_preserve_source_ordering() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = errno_cell(&mut owner, &mut guest);
    call(&mut owner, &mut guest, Service::PosixSemInit, &[ARG, 0, 1]).unwrap();
    for (sec, ns) in [(-1_i64, 0_i64), (0, -1), (0, 1_000_000_000)] {
        guest.put(64, &sec.to_le_bytes());
        guest.put(72, &ns.to_le_bytes());
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::PosixSemTimedWait,
                &[ARG, ARG + 64]
            ),
            Ok(u64::from(u32::MAX))
        );
        assert_eq!(guest.get(errno, 4), 22_u32.to_le_bytes());
        assert_eq!(count(&mut owner, &mut guest, ARG), 1);
    }
    guest.put(64, &[0; 16]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemTimedWait,
            &[ARG, ARG + 64]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemTimedWait,
            &[ARG, ARG + 64]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(guest.get(errno, 4), 60_u32.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemRelativeWait,
            &[ARG, 1 << 32]
        ),
        Ok(u64::from(u32::MAX))
    );
    let before = guest.get(ARG, 16);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemRelativeWait,
            &[ARG, 1]
        ),
        Err(Failure::Unsupported("POSIX semaphore blocking wait"))
    );
    assert_eq!(guest.get(ARG, 16), before);
    let direct = owner.invoke_library(
        Service::PthreadSemTimedWait,
        CallArgs([ARG, 0, 0, 0, 0, 0]),
        &mut guest,
    );
    assert_eq!(direct, Ok(0x8002_003c));
    assert_eq!(guest.get(errno, 4), 60_u32.to_le_bytes());
    call(
        &mut owner,
        &mut guest,
        Service::PosixSemInit,
        &[ARG, 0, i32::MAX as u64],
    )
    .unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemPost, &[ARG]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(guest.get(errno, 4), 84_u32.to_le_bytes());
    assert_eq!(count(&mut owner, &mut guest, ARG), i32::MAX as u32);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemInit,
            &[ARG, 0, 1_u64 << 31]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(count(&mut owner, &mut guest, ARG), i32::MAX as u32);
}

#[test]
fn pthread_semaphore_limits_refuse_before_output_and_reinit_reuses_record() {
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    call(
        &mut owner,
        &mut guest,
        Service::PthreadSemInit,
        &[ARG, 0, 1],
    )
    .unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::PthreadSemInit,
        &[ARG, 0, 2],
    )
    .unwrap();
    guest.put(64, &[0xa5; 16]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PthreadSemInit,
            &[ARG + 64, 0, 1]
        ),
        Err(Failure::Limit("handles"))
    );
    assert_eq!(guest.get(ARG + 64, 16), [0xa5; 16]);
    call(&mut owner, &mut guest, Service::PthreadSemDestroy, &[ARG]).unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::PthreadSemInit,
        &[ARG + 64, 0, 1],
    )
    .unwrap();
    for limits in [
        Limits {
            max_bytes: 15,
            ..Default::default()
        },
        Limits {
            max_work: 1,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        guest.put(0, &[0xa5; 16]);
        assert!(matches!(
            call(
                &mut owner,
                &mut guest,
                Service::PthreadSemInit,
                &[ARG, 0, 1]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.get(ARG, 16), [0xa5; 16]);
        assert_eq!(owner.usage().handles_live, 0);
    }
}
