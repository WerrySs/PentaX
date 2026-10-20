// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;

#[test]
fn content_integer_parameters_follow_source_sku_missing_metadata_and_exact_output_without_events() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for id in 0..=4 {
        guest.put(0, &[0xa5; 16]);
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                AppContentAppParamGetInt,
                (7 << 32) | id,
                ARG + 4
            ),
            Ok(0)
        );
        assert_eq!(guest.get(ARG, 4), [0xa5; 4]);
        assert_eq!(
            guest.get(ARG + 4, 4),
            (if id == 0 { 3_i32 } else { 0 }).to_le_bytes()
        );
        assert_eq!(guest.get(ARG + 8, 8), [0xa5; 8]);
        assert!(!owner.application_content_initialized());
        assert_eq!(count(&mut owner, &mut guest), 0);
    }
    assert_eq!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, 0),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, AppContentAppParamGetInt, 0, ARG),
        Ok(0)
    );
    assert_eq!(count(&mut owner, &mut guest), 1);
    for (module, library) in [
        ("AppContentUtil", "AppContent"),
        ("libSceAppContentUtil", "AppContent"),
        ("AppContentUtil", "libSceAppContent"),
        ("libSceAppContentUtil", "libSceAppContent"),
    ] {
        assert_eq!(
            super::resolve(module, (1, 1), library, 1, "99b82IKXpH4"),
            Some(AppContentAppParamGetInt)
        );
        assert_eq!(
            super::resolve(module, (1, 1), library, 2, "99b82IKXpH4"),
            None
        );
    }
}

#[test]
fn content_integer_parameter_unknown_branch_preserves_zero_prefix_and_refused_outputs_stay_typed() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 16]);
    assert_eq!(
        call(&mut owner, &mut guest, AppContentAppParamGetInt, 5, ARG + 4),
        Err(Failure::Unsupported(
            "application-content integer parameter"
        ))
    );
    assert_eq!(guest.get(ARG + 4, 4), [0; 4]);
    assert_eq!(guest.get(ARG + 8, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, AppContentAppParamGetInt, 0, 0),
        Err(Failure::Unsupported(
            "null application-content integer output"
        ))
    );
    guest.put(0, &[0xa5; 16]);
    guest.refuse = true;
    assert_eq!(
        call(&mut owner, &mut guest, AppContentAppParamGetInt, 0, ARG + 4),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG, 16), [0xa5; 16]);
    guest.refuse = false;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            AppContentAppParamGetInt,
            0,
            ARG + PAGE - 2
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    let mut limited = kernel(Limits {
        max_bytes: 3,
        ..Default::default()
    });
    assert!(matches!(
        call(
            &mut limited,
            &mut guest,
            AppContentAppParamGetInt,
            0,
            ARG + 4
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG, 16), [0xa5; 16]);
}

fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    a: u64,
    b: u64,
) -> Result<u64, Failure> {
    owner.invoke_library(service, CallArgs([a, b, 0, 0, 0, 0]), guest)
}
fn count(owner: &mut Kernel, guest: &mut TestGuest) -> i32 {
    assert_eq!(call(owner, guest, SystemGetStatus, ARG + 256, 0), Ok(0));
    i32::from_le_bytes(guest.get(ARG + 256, 4).try_into().unwrap())
}

#[test]
fn content_initialization_clears_exact_boot_span_and_enqueues_every_notification() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 56]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            AppContentInitialize,
            u64::MAX,
            ARG + 8
        ),
        Ok(0)
    );
    assert!(owner.application_content_initialized());
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(guest.get(ARG + 8, 40), [0; 40]);
    assert_eq!(guest.get(ARG + 48, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, 0),
        Ok(0)
    );
    assert_eq!(count(&mut owner, &mut guest), 2);
    for remaining in [1, 0] {
        guest.put(512, &vec![0xa5; 8204]);
        assert_eq!(
            call(&mut owner, &mut guest, SystemReceiveEvent, ARG + 512, 0),
            Ok(0)
        );
        assert_eq!(guest.get(ARG + 512, 4), 0x1000_0003_i32.to_le_bytes());
        assert_eq!(guest.get(ARG + 516, 8192), vec![0; 8192]);
        assert_eq!(guest.get(ARG + 512 + 8196, 8), [0xa5; 8]);
        assert_eq!(count(&mut owner, &mut guest), remaining);
    }
    assert_eq!(
        call(&mut owner, &mut guest, SystemReceiveEvent, ARG + 512, 0),
        Ok(0x80a1_0004)
    );
    assert_eq!(guest.get(ARG + 512, 4), (-1_i32).to_le_bytes());
    assert_eq!(guest.get(ARG + 516, 8192), vec![0; 8192]);
    assert_eq!(count(&mut kernel(Default::default()), &mut guest), 0);
}

#[test]
fn content_and_event_failures_preserve_output_receipt_and_queue_until_retry() {
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa5; 40]);
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, ARG),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert!(!owner.application_content_initialized());
    assert_eq!(guest.get(ARG, 40), [0xa5; 40]);
    assert_eq!(count(&mut owner, &mut guest), 0);
    assert_eq!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, 0),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, ARG),
        Err(Failure::Limit("system events"))
    );
    assert_eq!(guest.get(ARG, 40), [0xa5; 40]);
    assert_eq!(count(&mut owner, &mut guest), 1);
    guest.put(512, &vec![0xa5; 8196]);
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, SystemReceiveEvent, ARG + 512, 0),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.get(ARG + 512, 8196), vec![0xa5; 8196]);
    assert_eq!(count(&mut owner, &mut guest), 1);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            SystemReceiveEvent,
            ARG + PAGE - 4,
            0
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(count(&mut owner, &mut guest), 1);
    assert_eq!(
        call(&mut owner, &mut guest, SystemReceiveEvent, ARG + 512, 0),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, ARG),
        Ok(0)
    );
    assert_eq!(count(&mut owner, &mut guest), 1);
}

#[test]
fn event_status_has_exact_layout_null_codes_and_scratch_refusal_is_non_consuming() {
    let mut owner = kernel(Limits {
        max_scratch_bytes: 512,
        max_name_bytes: 64,
        max_io_bytes: 512,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, 0),
        Ok(0)
    );
    guest.put(256, &[0xa5; 144]);
    assert_eq!(count(&mut owner, &mut guest), 1);
    assert_eq!(guest.get(ARG + 260, 132), [0; 132]);
    assert_eq!(guest.get(ARG + 392, 8), [0xa5; 8]);
    for service in [SystemGetStatus, SystemReceiveEvent] {
        assert_eq!(call(&mut owner, &mut guest, service, 0, 0), Ok(0x80a1_0003));
    }
    assert_eq!(
        call(&mut owner, &mut guest, SystemReceiveEvent, ARG + 512, 0),
        Err(Failure::Limit("scratch"))
    );
    assert_eq!(count(&mut owner, &mut guest), 1);
    let used = owner.usage().work;
    guest.put(0, &[0xa5; 40]);
    owner
        .budget
        .work(Limits::default().max_work - used)
        .unwrap();
    assert!(matches!(
        call(&mut owner, &mut guest, AppContentInitialize, 0, ARG),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG, 40), [0xa5; 40]);
}
