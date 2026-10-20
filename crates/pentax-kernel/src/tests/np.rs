// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;

const SIGNED_OUT: u64 = 0x8055_0006;
const INVALID_ARGUMENT: u64 = 0x8055_0003;
const NOT_FOUND: u64 = 0x8055_0014;
const ABORTED: u64 = 0x8055_0012;
const INVALID_ID: u64 = 0x8055_0015;
fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn asynchronous(owner: &mut Kernel, guest: &mut TestGuest) -> u64 {
    guest.put(128, &24_u64.to_le_bytes());
    guest.put(136, &u64::MAX.to_le_bytes());
    guest.put(144, &(-731_i32).to_le_bytes());
    call(owner, guest, NpCreateAsyncRequest, &[ARG + 128]).unwrap()
}

#[test]
fn np_account_queries_report_offline_state_with_exact_output_spans_and_source_null_rules() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for (service, width, result, value) in [
        (NpGetId, 36, SIGNED_OUT, 0_u8),
        (NpGetOnlineId, 20, SIGNED_OUT, 0),
        (NpGetAccountId, 8, SIGNED_OUT, 0),
        (NpGetAccountCountry, 4, SIGNED_OUT, 0),
        (NpGetState, 4, 0, 1),
        (NpGetReachabilityState, 4, 0, 0),
        (NpHasSignedUp, 1, 0, 0),
    ] {
        guest.put(0, &[0xa5; 48]);
        assert_eq!(
            call(&mut owner, &mut guest, service, &[u64::MAX, ARG + 4]),
            Ok(result)
        );
        let mut expected = vec![0; width];
        expected[0] = value;
        assert_eq!(guest.get(ARG + 4, width), expected);
        assert_eq!(guest.get(ARG, 4), [0xa5; 4]);
        assert_eq!(guest.get(ARG + 4 + width as u64, 4), [0xa5; 4]);
    }
    for service in [
        NpGetId,
        NpGetOnlineId,
        NpGetAccountId,
        NpGetAccountCountry,
        NpGetState,
    ] {
        assert!(matches!(
            call(&mut owner, &mut guest, service, &[0, 0]),
            Err(Failure::Unsupported(_))
        ));
    }
    for service in [NpHasSignedUp, NpGetReachabilityState] {
        assert_eq!(
            call(&mut owner, &mut guest, service, &[0, 0]),
            Ok(INVALID_ARGUMENT)
        );
    }
    guest.put(0, &[0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, NpGetAccountAge, &[999, 0, ARG]),
        Ok(SIGNED_OUT)
    );
    assert_eq!(
        guest.get(ARG, 8),
        [0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]
    );
    assert_eq!(
        call(&mut owner, &mut guest, NpGetAccountAge, &[0, 0, ARG]),
        Ok(INVALID_ARGUMENT)
    );
}

#[test]
fn np_requests_have_real_completion_poll_abort_delete_and_lowest_free_slot_reuse() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let sync = call(&mut owner, &mut guest, NpCreateRequest, &[]).unwrap();
    let asynchronous = asynchronous(&mut owner, &mut guest);
    assert_eq!((sync, asynchronous, owner.usage().handles_live), (1, 2, 2));
    guest.put(0, &[0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[asynchronous, ARG]),
        Ok(INVALID_ID)
    );
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, NpCheckReachability, &[sync, 0]),
        Ok(SIGNED_OUT)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckReachability,
            &[asynchronous, u64::MAX]
        ),
        Ok(0)
    );
    for _ in 0..2 {
        assert_eq!(
            call(&mut owner, &mut guest, NpPollAsync, &[asynchronous, ARG]),
            Ok(0)
        );
        assert_eq!(guest.u64(0), 0xa5a5_a5a5_8055_0006);
    }
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, NpPollAsync, &[asynchronous, ARG]),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[asynchronous, ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0xa5a5_a5a5_8055_0006);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckReachability,
            &[asynchronous, 0]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[asynchronous, ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0xa5a5_a5a5_8055_0003);
    assert_eq!(
        call(&mut owner, &mut guest, NpAbortRequest, &[asynchronous]),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckReachability,
            &[asynchronous, 0]
        ),
        Ok(ABORTED)
    );
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[asynchronous, ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0xa5a5_a5a5_8055_0012);
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[sync, ARG]),
        Ok(INVALID_ID)
    );
    assert_eq!(
        call(&mut owner, &mut guest, NpDeleteRequest, &[sync]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, NpDeleteRequest, &[sync]),
        Ok(NOT_FOUND)
    );
    assert_eq!(call(&mut owner, &mut guest, NpCreateRequest, &[]), Ok(sync));
    assert_eq!(owner.usage().handles_live, 2);
    assert_eq!(
        call(
            &mut kernel(Default::default()),
            &mut guest,
            NpAbortRequest,
            &[asynchronous]
        ),
        Ok(NOT_FOUND)
    );
}

#[test]
fn np_availability_output_order_and_premium_failures_preserve_actual_request_state() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, NpCheckPremium, &[999, 0, 0]),
        Ok(INVALID_ARGUMENT)
    );
    guest.put(64, &55_u64.to_le_bytes());
    guest.put(256, &[0xa5; 48]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckPremium,
            &[999, ARG + 64, ARG + 256]
        ),
        Ok(0x8055_0011)
    );
    guest.put(64, &56_u64.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckPremium,
            &[999, ARG + 64, ARG + 256]
        ),
        Ok(NOT_FOUND)
    );
    assert_eq!(guest.get(ARG + 256, 48), [0xa5; 48]);
    guest.put(0, &[0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckAvailability,
            &[999, u64::MAX, ARG]
        ),
        Ok(NOT_FOUND)
    );
    assert_eq!(guest.u64(0), 0xa5a5_a5a5_0000_0000);
    let id = asynchronous(&mut owner, &mut guest);
    guest.put(0, &[0xa5; 8]);
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, NpCheckAvailability, &[id, ARG, ARG]),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[id, ARG]),
        Ok(INVALID_ID)
    );
    guest.put(64, &56_u64.to_le_bytes());
    guest.put(256, &[0xa5; 48]);
    guest.refuse = true;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            NpCheckPremium,
            &[id, ARG + 64, ARG + 256]
        ),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(
        call(&mut owner, &mut guest, NpPollAsync, &[id, ARG]),
        Ok(INVALID_ID)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckPremium,
            &[id, ARG + 64, ARG + 256]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 256, 33), [0; 33]);
    assert_eq!(guest.get(ARG + 289, 15), [0xa5; 15]);
    guest.put(256, &[0xa5; 48]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpCheckPremium,
            &[id, ARG + 64, ARG + 256]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(guest.get(ARG + 256, 48), [0xa5; 48]);
    assert_eq!(call(&mut owner, &mut guest, NpPollAsync, &[id, ARG]), Ok(0));
    assert_eq!(guest.u64(0), 0xa5a5_a5a5_8055_0003);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            NpGetAccountLanguage,
            &[id, 0, u64::MAX]
        ),
        Ok(SIGNED_OUT)
    );
    assert_eq!(
        call(&mut owner, &mut guest, NpGetAccountLanguage, &[999, 0, ARG]),
        Ok(NOT_FOUND)
    );
}

#[test]
fn np_request_limits_and_parameter_refusal_do_not_publish_or_leak_handles() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, NpCreateAsyncRequest, &[0]),
        Ok(INVALID_ARGUMENT)
    );
    guest.put(0, &23_u64.to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, NpCreateAsyncRequest, &[ARG]),
        Ok(0x8055_0011)
    );
    guest.put(PAGE - 8, &24_u64.to_le_bytes());
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            NpCreateAsyncRequest,
            &[ARG + PAGE - 8]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(owner.usage().handles_live, 0);
    for expected in 1..=128 {
        assert_eq!(
            call(&mut owner, &mut guest, NpCreateRequest, &[]),
            Ok(expected)
        );
    }
    assert_eq!(
        call(&mut owner, &mut guest, NpCreateRequest, &[]),
        Ok(0x8055_0013)
    );
    assert_eq!(owner.usage().handles_live, 128);
    assert_eq!(call(&mut owner, &mut guest, NpDeleteRequest, &[17]), Ok(0));
    assert_eq!(call(&mut owner, &mut guest, NpCreateRequest, &[]), Ok(17));
    let mut limited = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    assert_eq!(call(&mut limited, &mut guest, NpCreateRequest, &[]), Ok(1));
    assert!(matches!(
        call(&mut limited, &mut guest, NpCreateRequest, &[]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(limited.usage().handles_live, 1);
    let mut exhausted = kernel(Limits {
        max_work: 0,
        ..Default::default()
    });
    assert!(matches!(
        call(&mut exhausted, &mut guest, NpCreateRequest, &[]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(exhausted.usage().handles_live, 0);
}
