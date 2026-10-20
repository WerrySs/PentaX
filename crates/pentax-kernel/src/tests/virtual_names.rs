// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn virtual_name_calls_validate_exact_names_before_requiring_owner_integration() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        owner.invoke_library(
            Service::SetVirtualRangeName,
            CallArgs([u64::MAX, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0x8002_000e)
    );
    guest.put(0, &[0x61; 32]);
    assert_eq!(
        owner.invoke_library(
            Service::SetVirtualRangeName,
            CallArgs([0, 0, ARG, 0, 0, 0]),
            &mut guest
        ),
        Ok(0x8002_003f)
    );
    guest.put(0, &[0xff, 0x80, 0]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SetVirtualRangeName,
            &[ARG, PAGE, ARG]
        ),
        Err(Failure::Unsupported("bound virtual range names"))
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ClearVirtualRangeName,
            &[ARG, PAGE]
        ),
        Err(Failure::Unsupported("bound virtual range names"))
    );
    assert_eq!(guest.get(ARG, 3), [0xff, 0x80, 0]);
    assert_eq!(owner.usage().bytes_written, 0);
    for (nid, service) in [
        ("DGMG3JshrZU", Service::SetVirtualRangeName),
        ("mkgXxsoxWHg", Service::ClearVirtualRangeName),
    ] {
        assert_eq!(
            resolve("libkernel", (1, 1), "libkernel", 1, nid),
            Some(service)
        );
        assert_eq!(resolve("libkernel", (1, 2), "libkernel", 1, nid), None);
        assert_eq!(resolve("libkernel", (1, 1), "Posix", 1, nid), None);
    }
}

#[test]
fn virtual_name_guest_limits_remain_typed_failures_without_output() {
    for limits in [
        Limits {
            max_bytes: 2,
            ..Default::default()
        },
        Limits {
            max_name_bytes: 2,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        let mut guest = TestGuest::new();
        guest.put(0, b"abc\0");
        assert!(matches!(
            call(
                &mut owner,
                &mut guest,
                Service::SetVirtualRangeName,
                &[ARG, PAGE, ARG]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.get(ARG, 4), b"abc\0");
        assert_eq!(owner.usage().bytes_written, 0);
    }
}
