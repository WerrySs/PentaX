// SPDX-License-Identifier: GPL-2.0-only
//! Original checked outputs and state-independence for the public WS1 getter.

use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{
    CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Limits, MemoryEffect, Service,
    UserConfig,
};

fn call(
    owner: &mut crate::Kernel,
    guest: &mut impl GuestAccess,
    id: u64,
    output: u64,
) -> Result<u64, Failure> {
    owner.invoke_library(
        Service::UserGetPlatformPrivacyWs1,
        CallArgs([id, output, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
        guest,
    )
}

struct NoAccess;
impl GuestAccess for NoAccess {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("unexpected WS1 read");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("unexpected WS1 write");
    }
}

#[test]
fn privacy_null_is_a_normal_int32_error_and_ignores_user_identity_without_state_changes() {
    let mut owner = kernel(Limits::default());
    for id in [0, 1, u64::MAX, 0x1234_5678_ffff_ffff] {
        assert_eq!(call(&mut owner, &mut NoAccess, id, 0), Ok(0x8096_0005));
    }
    assert!(!owner.user_service_initialized());
    owner
        .configure_user(UserConfig {
            id: 37,
            ..UserConfig::default()
        })
        .unwrap();
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 16]);
    assert_eq!(call(&mut owner, &mut guest, u64::MAX, ARG + 3), Ok(0));
    // Successful WS1 output must not freeze the otherwise unobserved profile.
    owner
        .configure_user(UserConfig {
            id: 39,
            ..UserConfig::default()
        })
        .unwrap();
    assert!(!owner.user_service_initialized());
    assert_eq!(
        owner.invoke_library(
            Service::UserGetEvent,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 39_u64 << 32);
    assert_eq!(call(&mut owner, &mut guest, 1, ARG + 8), Ok(0));
    assert_eq!(
        owner.invoke_library(
            Service::UserGetEvent,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0x8096_0007)
    );
    assert_eq!(guest.u64(0), 39_u64 << 32);
    assert!(!owner.user_service_initialized());
    assert_eq!(
        owner.invoke_library(
            Service::UserInitialize2,
            CallArgs([u64::MAX; 6]),
            &mut guest
        ),
        Ok(0)
    );
    assert_eq!(call(&mut owner, &mut guest, 0, ARG + 8), Ok(0));
    assert!(owner.user_service_initialized());
    assert_eq!(
        owner.invoke_library(
            Service::UserGetEvent,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0x8096_0007)
    );
}

#[test]
fn privacy_writes_exactly_four_bytes_at_unaligned_and_page_crossing_outputs() {
    for offset in [0, 1, PAGE - 4, PAGE - 2, PAGE] {
        let mut owner = kernel(Limits::default());
        let mut guest = TestGuest::new();
        guest.regions[0].length = 2 * PAGE;
        guest.regions[0].bytes = vec![0xa5; (2 * PAGE) as usize];
        assert_eq!(
            call(&mut owner, &mut guest, 0xffff_ffff_0000_0001, ARG + offset),
            Ok(0)
        );
        let mut expected = vec![0xa5; (2 * PAGE) as usize];
        expected[offset as usize..offset as usize + 4].fill(0);
        assert_eq!(guest.regions[0].bytes, expected);
        assert_eq!(
            (
                owner.usage().attempts,
                owner.usage().work,
                owner.usage().bytes_read,
                owner.usage().bytes_written
            ),
            (1, 6, 0, 4)
        );
    }
}

#[test]
fn privacy_backing_and_budget_failures_preserve_the_complete_output_transaction() {
    for (output, readonly, expected) in [
        (ARG + PAGE, false, GuestError::Unmapped),
        (ARG + PAGE - 2, false, GuestError::Unmapped),
        (ARG + 1, true, GuestError::WriteDenied),
        (u64::MAX - 1, false, GuestError::AddressOverflow),
    ] {
        let mut owner = kernel(Limits::default());
        let mut guest = TestGuest::new();
        guest.regions[0].bytes.fill(0xa5);
        if readonly {
            guest.regions[0].protection = 1;
        }
        assert_eq!(
            call(&mut owner, &mut guest, u64::MAX, output),
            Err(Failure::Guest(expected))
        );
        assert!(guest.regions[0].bytes.iter().all(|byte| *byte == 0xa5));
        assert!(!owner.user_service_initialized());
        owner
            .configure_user(UserConfig {
                id: 37,
                ..UserConfig::default()
            })
            .unwrap();
    }
    for (limits, label, attempts, work) in [
        (
            Limits {
                max_attempts: 0,
                ..Limits::default()
            },
            "attempts",
            0,
            0,
        ),
        (
            Limits {
                max_work: 1,
                ..Limits::default()
            },
            "work",
            1,
            1,
        ),
        (
            Limits {
                max_work: 5,
                ..Limits::default()
            },
            "work",
            1,
            2,
        ),
        (
            Limits {
                max_bytes: 3,
                ..Limits::default()
            },
            "guest bytes",
            1,
            2,
        ),
    ] {
        let mut owner = kernel(limits);
        assert_eq!(
            call(&mut owner, &mut NoAccess, 1, ARG),
            Err(Failure::Limit(label))
        );
        assert_eq!(
            (
                owner.usage().attempts,
                owner.usage().work,
                owner.usage().bytes_read,
                owner.usage().bytes_written
            ),
            (attempts, work, 0, 0)
        );
        assert!(!owner.user_service_initialized());
    }
}
