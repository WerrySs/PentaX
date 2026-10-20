// SPDX-License-Identifier: GPL-2.0-only
//! Original checks of the pinned stateless, logging-only Share profile.

use super::*;

struct NoAccess;
impl GuestAccess for NoAccess {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("Share accessed an ignored argument");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("Share mutated guest state");
    }
}

#[test]
fn share_lifecycle_opaque_words_and_winning_permit_retain_no_heap_thread_or_device() {
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Limits::default()
    });
    let census = owner.handle_usage();
    let mut expected = owner.usage();
    // Terminate/permit before Init and repeated Init are valid in the source.
    for service in [
        Service::ShareTerminate,
        Service::ShareFeaturePermit,
        Service::ShareInitialize,
        Service::ShareInitialize,
    ] {
        for word in [0, u64::MAX, 0x8000_0000, 0x1234_5678_0000_0000] {
            assert_eq!(
                owner.invoke_library(service, CallArgs([word; 6]), &mut NoAccess),
                Ok(
                    if service == Service::ShareFeaturePermit && word as u32 == 0 {
                        0x8196_0002
                    } else {
                        0
                    }
                )
            );
            expected.attempts += 1;
            expected.work += 2;
            assert_eq!(owner.usage(), expected);
            assert_eq!(owner.handle_usage(), census);
        }
    }
}

#[test]
fn share_obeys_work_and_attempt_limits_without_errno_or_hidden_output() {
    for service in [
        Service::ShareInitialize,
        Service::ShareFeaturePermit,
        Service::ShareTerminate,
    ] {
        let mut work = kernel(Limits {
            max_work: 1,
            ..Limits::default()
        });
        assert_eq!(
            work.invoke_library(service, CallArgs([u64::MAX; 6]), &mut NoAccess),
            Err(Failure::Limit("work"))
        );
        assert_eq!(
            (
                work.usage().attempts,
                work.usage().work,
                work.usage().bytes_written
            ),
            (1, 1, 0)
        );
        let mut attempts = kernel(Limits {
            max_attempts: 1,
            ..Limits::default()
        });
        let mut guest = TestGuest::new();
        assert_eq!(
            attempts.invoke_library(service, CallArgs([0; 6]), &mut guest),
            Ok(if service == Service::ShareFeaturePermit {
                0x8196_0002
            } else {
                0
            })
        );
        assert_eq!(
            attempts.invoke_library(service, CallArgs([0; 6]), &mut guest),
            Err(Failure::Limit("attempts"))
        );
    }
    let mut owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let errno = owner
        .invoke_library(Service::ErrnoLocation, CallArgs([0; 6]), &mut guest)
        .unwrap();
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address: errno,
                bytes: &37_u32.to_le_bytes(),
            }],
        )
        .unwrap();
    let usage = owner.usage();
    for service in [
        Service::ShareInitialize,
        Service::ShareFeaturePermit,
        Service::ShareTerminate,
    ] {
        assert_eq!(
            owner.invoke_library(service, CallArgs([u64::MAX; 6]), &mut guest),
            Ok(0)
        );
        assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    }
    assert_eq!(
        (
            owner.usage().bytes_read,
            owner.usage().bytes_written,
            owner.usage().handles_live
        ),
        (usage.bytes_read, usage.bytes_written, usage.handles_live)
    );
}
