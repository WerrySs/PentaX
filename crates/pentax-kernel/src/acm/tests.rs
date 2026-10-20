// SPDX-License-Identifier: GPL-2.0-only
//! Original creation-only contexts and atomic refusal fixtures.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{GuestError, Limits, MemoryEffect, Service};

fn create(owner: &mut Kernel, guest: &mut TestGuest, output: u64) -> Result<u64, Failure> {
    owner.invoke_library(
        Service::AcmContextCreate,
        CallArgs([output, u64::MAX, 7, 8, 9, 10]),
        guest,
    )
}

#[test]
fn create_writes_exactly_four_bytes_and_keeps_process_owned_monotonic_handles() {
    let mut owner = kernel(Limits {
        max_handles: 3,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 16]);
    assert_eq!(create(&mut owner, &mut guest, ARG + 1), Ok(0));
    assert_eq!(create(&mut owner, &mut guest, ARG + 7), Ok(0));
    let mut expected = [0xa5; 16];
    expected[1..5].copy_from_slice(&1u32.to_le_bytes());
    expected[7..11].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(guest.get(ARG, 16), expected);
    let census = owner.handle_usage();
    assert_eq!(
        (census.live, census.accounted(), census.acm, census.memory),
        (2, 2, 2, 0)
    );
    assert_eq!(
        (owner.usage().attempts, owner.usage().bytes_written),
        (2, 8)
    );
    // The final four bytes of a valid mapping are a complete output, with no
    // invented alignment or read permission requirement.
    guest
        .apply(
            MemoryEffect::Protect {
                address: ARG,
                length: PAGE,
                protection: 2,
            },
            &[],
        )
        .unwrap();
    assert_eq!(create(&mut owner, &mut guest, ARG + PAGE - 4), Ok(0));
    guest
        .apply(
            MemoryEffect::Protect {
                address: ARG,
                length: PAGE,
                protection: 3,
            },
            &[],
        )
        .unwrap();
    assert_eq!(guest.get(ARG + PAGE - 4, 4), 3u32.to_le_bytes());
    let mut independent = kernel(Default::default());
    assert_eq!(create(&mut independent, &mut guest, ARG), Ok(0));
    assert_eq!(guest.get(ARG, 4), 1u32.to_le_bytes());
    assert_eq!(independent.handle_usage().acm, 1);
}

#[test]
fn failed_live_output_and_null_do_not_publish_contexts_or_consume_identifiers() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, &[0x9c; 16]);
    assert_eq!(
        create(&mut owner, &mut guest, 0),
        Err(Failure::Unsupported("null ACM context output"))
    );
    for address in [ARG + PAGE - 3, u64::MAX - 2] {
        assert!(matches!(
            create(&mut owner, &mut guest, address),
            Err(Failure::Guest(_))
        ));
    }
    guest
        .apply(
            MemoryEffect::Protect {
                address: ARG,
                length: PAGE,
                protection: 1,
            },
            &[],
        )
        .unwrap();
    assert_eq!(
        create(&mut owner, &mut guest, ARG),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest
        .apply(
            MemoryEffect::Protect {
                address: ARG,
                length: PAGE,
                protection: 3,
            },
            &[],
        )
        .unwrap();
    guest.refuse = true;
    assert!(matches!(
        create(&mut owner, &mut guest, ARG),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.get(ARG, 16), [0x9c; 16]);
    assert_eq!(
        (
            owner.acm.next,
            owner.acm.contexts,
            owner.handle_usage().accounted()
        ),
        (1, 0, 0)
    );
    assert_eq!(create(&mut owner, &mut guest, ARG), Ok(0));
    assert_eq!(guest.get(ARG, 4), 1u32.to_le_bytes());
    assert_eq!(owner.handle_usage().acm, 1);
}

#[test]
fn quota_work_bytes_and_attempt_limits_refuse_before_output_and_shared_handle_publication() {
    for limits in [
        Limits {
            max_handles: 0,
            ..Default::default()
        },
        Limits {
            max_attempts: 0,
            ..Default::default()
        },
        Limits {
            max_work: 5,
            ..Default::default()
        },
        Limits {
            max_bytes: 3,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        let mut guest = TestGuest::new();
        guest.put(0, &[0x6d; 8]);
        assert!(matches!(
            create(&mut owner, &mut guest, ARG),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.get(ARG, 8), [0x6d; 8]);
        assert_eq!(
            (
                owner.acm.next,
                owner.acm.contexts,
                owner.handle_usage().live,
                owner.handle_usage().acm
            ),
            (1, 0, 0, 0)
        );
    }
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(
        owner.invoke_library(
            Service::SysmoduleLoad,
            CallArgs([7, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0)
    );
    assert_eq!(
        create(&mut owner, &mut guest, ARG),
        Err(Failure::Limit("handles"))
    );
    assert_eq!(owner.acm.next, 1);
    assert_eq!(
        owner.invoke_library(
            Service::SysmoduleUnload,
            CallArgs([7, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0)
    );
    assert_eq!(create(&mut owner, &mut guest, ARG), Ok(0));
    assert_eq!(
        (
            owner.handle_usage().live,
            owner.handle_usage().accounted(),
            owner.handle_usage().acm
        ),
        (1, 1, 1)
    );
    assert_eq!(guest.get(ARG, 4), 1u32.to_le_bytes());
}

#[test]
fn last_u32_identifier_is_issued_once_and_exhaustion_never_wraps_or_writes() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.acm.next = u64::from(u32::MAX);
    assert_eq!(create(&mut owner, &mut guest, ARG), Ok(0));
    assert_eq!(guest.get(ARG, 4), u32::MAX.to_le_bytes());
    guest.put(0, &[0x42; 4]);
    assert_eq!(
        create(&mut owner, &mut guest, ARG),
        Err(Failure::Limit("ACM context identifiers"))
    );
    assert_eq!(guest.get(ARG, 4), [0x42; 4]);
    assert_eq!(owner.acm.next, u64::from(u32::MAX) + 1);
    assert_eq!(
        (
            owner.handle_usage().live,
            owner.handle_usage().accounted(),
            owner.handle_usage().acm
        ),
        (1, 1, 1)
    );
}
