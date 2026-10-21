// SPDX-License-Identifier: GPL-2.0-only
//! Original error-precedence, signed ABI, and exact checked-output consumers.
use super::*;

fn query(
    kernel: &mut Kernel,
    guest: &mut impl GuestAccess,
    user: u64,
    output: u64,
) -> Result<u64, Failure> {
    kernel.invoke_library(
        Service::ConvertKeycodeGetImeKeyboardType,
        CallArgs([user, output, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
        guest,
    )
}

struct NoAccess;
impl GuestAccess for NoAccess {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("unexpected keyboard read")
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("unexpected keyboard write")
    }
}

#[test]
fn keyboard_type_preserves_null_precedence_and_signed_low_word_without_touching_user_state() {
    let mut owner = kernel(Limits::default());
    for user in [0, 1, u64::MAX, 0xa5a5_a5a5_8000_0000] {
        assert_eq!(query(&mut owner, &mut NoAccess, user, 0), Ok(0x80bc_0031));
    }
    for user in [u64::MAX, 0x8000_0000, 0x1234_5678_ffff_ffff] {
        assert_eq!(
            query(&mut owner, &mut NoAccess, user, u64::MAX),
            Ok(0x80bc_0010)
        );
    }
    for user in [0, 1, 0x7fff_ffff, 0xffff_ffff_0000_0001] {
        let mut guest = TestGuest::new();
        guest.put(0, &[0xa5; 16]);
        assert_eq!(query(&mut owner, &mut guest, user, ARG + 3), Ok(0));
        assert_eq!(
            &guest.regions[0].bytes[..16],
            &[
                0xa5, 0xa5, 0xa5, 0, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5
            ]
        );
    }
    assert!(!owner.user_service_initialized());
    owner
        .configure_user(UserConfig {
            id: 37,
            ..Default::default()
        })
        .unwrap();
}

#[test]
fn keyboard_type_checks_the_entire_four_byte_span_and_quota_before_any_output() {
    for (offset, readonly) in [(PAGE - 2, false), (1, true)] {
        let mut owner = kernel(Limits::default());
        let mut guest = TestGuest::new();
        guest.regions[0].bytes.fill(0xa5);
        if readonly {
            guest.regions[0].protection = 1;
        }
        assert_eq!(
            query(&mut owner, &mut guest, 1, ARG + offset),
            Err(Failure::Guest(GuestError::WriteDenied))
        );
        assert!(guest.regions[0].bytes.iter().all(|b| *b == 0xa5));
    }
    for limits in [
        Limits {
            max_bytes: 3,
            ..Default::default()
        },
        Limits {
            max_work: 5,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            query(&mut kernel(limits), &mut NoAccess, 1, ARG),
            Err(Failure::Limit(_))
        ));
    }
    let mut owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.regions[0].length = 2 * PAGE;
    guest.regions[0].bytes = vec![0xa5; (2 * PAGE) as usize];
    assert_eq!(query(&mut owner, &mut guest, 1, ARG + PAGE - 2), Ok(0));
    let mut expected = vec![0xa5; (2 * PAGE) as usize];
    expected[PAGE as usize - 2..PAGE as usize + 2].fill(0);
    assert_eq!(guest.regions[0].bytes, expected);
    assert_eq!(owner.usage().bytes_written, 4);
}
