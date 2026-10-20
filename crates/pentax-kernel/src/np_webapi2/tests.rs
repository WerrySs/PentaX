// SPDX-License-Identifier: GPL-2.0-only
// Original offline NpWebApi2 fixtures. No network or guest game inputs.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest};
use crate::{Limits, Service::*};

fn limits() -> Limits {
    Limits {
        max_name_bytes: 32,
        max_io_bytes: 256,
        max_scratch_bytes: 4096,
        ..Default::default()
    }
}

fn fixture(policy: Limits) -> (NpWebApi2State, Budget, TestGuest) {
    let mut guest = TestGuest::new();
    guest.put(0, b"\xffgroup\0");
    guest.put(32, b"/local\0");
    guest.put(64, b"\xfeGET\0");
    guest.put(96, b"Content-Type\0");
    (
        NpWebApi2State::default(),
        Budget::new(policy).unwrap(),
        guest,
    )
}

fn call(
    state: &mut NpWebApi2State,
    budget: &mut Budget,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    state.invoke(service, CallArgs(args), guest, budget)
}

fn request(state: &mut NpWebApi2State, budget: &mut Budget, guest: &mut TestGuest) {
    assert_eq!(
        call(
            state,
            budget,
            guest,
            NpWebApi2CreateRequest,
            &[u64::MAX, ARG, ARG + 32, ARG + 64, u64::MAX, ARG + 128]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(128), 1000);
}

fn census(state: &NpWebApi2State, budget: &Budget) {
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn offline_lifetimes_keep_independent_ids_opaque_receipts_and_no_parent_cascades() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Initialize,
            &[u64::MAX, 0x4001]
        ),
        Ok(1)
    );
    assert_eq!(
        (state.contexts[0]._http_context, state.contexts[0].pool_size),
        (-1, 0x8000)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2PushEventCreateHandle,
            &[u64::MAX]
        ),
        Ok(1)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateUserContext,
            &[u64::MAX, 1 << 32]
        ),
        Ok(1001)
    );
    assert_eq!((state.users[0]._context, state.users[0]._user), (-1, 0));
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2PushEventCreateFilter,
            &[
                u64::MAX,
                1 << 32,
                u64::MAX,
                (7 << 32) | 9,
                u64::MAX - 1,
                u64::MAX
            ]
        ),
        Ok(1)
    );
    let filter = &state.filters[0];
    assert_eq!(
        (
            filter._context,
            filter._handle,
            filter._service_name,
            filter._service_label,
            filter._parameters,
            filter._parameter_count
        ),
        (-1, 0, u64::MAX, 9, u64::MAX - 1, u64::MAX)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2PushEventRegisterCallback,
            &[u64::MAX, 1 << 32, u64::MAX, u64::MAX - 2]
        ),
        Ok(1)
    );
    let callback = &state.callbacks[0];
    assert_eq!(
        (
            callback._user,
            callback._filter,
            callback._callback,
            callback._argument
        ),
        (-1, 0, u64::MAX, u64::MAX - 2)
    );
    request(&mut state, &mut budget, &mut guest);
    assert_eq!(
        (
            &state.requests[0].api_group[..],
            &state.requests[0].path[..],
            &state.requests[0].method[..]
        ),
        (&b"\xffgroup"[..], &b"/local"[..], &b"\xfeGET"[..])
    );
    assert_eq!(
        (
            state.requests[0]._user,
            state.requests[0]._content_parameter
        ),
        (-1, u64::MAX)
    );
    assert_eq!(state.retained_handle_count(), 6);
    census(&state, &budget);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CheckTimeout,
            &[]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Terminate,
            &[(31 << 32) | 1]
        ),
        Ok(0)
    );
    assert_eq!(state.retained_handle_count(), 5);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2DeleteUserContext,
            &[(31 << 32) | 1001]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2PushEventDeleteHandle,
            &[u64::MAX, (31 << 32) | 1]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2PushEventDeletePushContext,
            &[u64::MAX, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2AbortRequest,
            &[1000]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2DeleteRequest,
            &[1000]
        ),
        Ok(0)
    );
    assert_eq!(state.text_bytes, 0);
    assert_eq!(state.retained_handle_count(), 2); // Undelivered filter/callback receipts.
    census(&state, &budget);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Initialize,
            &[0, 0]
        ),
        Ok(2)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateUserContext,
            &[0, 0]
        ),
        Ok(1002)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2PushEventCreateHandle,
            &[0]
        ),
        Ok(2)
    );
    census(&state, &budget);
    assert_eq!(NpWebApi2State::default().retained_handle_count(), 0);
}

#[test]
fn pool_stats_preserve_padding_low_word_ids_and_source_error_precedence() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    for (args, result) in [
        ([0, 0], INVALID_ARGUMENT),
        ([0, u64::MAX], INVALID_CONTEXT),
        ([u64::MAX, u64::MAX], INVALID_CONTEXT),
        ([1, u64::MAX], CONTEXT_NOT_FOUND),
    ] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                NpWebApi2GetMemoryPoolStats,
                &args
            ),
            Ok(result)
        );
    }
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Initialize,
            &[0, u64::MAX]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!((state.next_context, state.retained_handle_count()), (1, 0));
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Initialize,
            &[u64::MAX, 1]
        ),
        Ok(1)
    );
    guest.put(160, &[0xa7; 40]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2GetMemoryPoolStats,
            &[(9 << 32) | 1, ARG + 164]
        ),
        Ok(0)
    );
    let mut expected = [0; 28];
    expected[..8].copy_from_slice(&0x4000u64.to_le_bytes());
    assert_eq!(guest.get(ARG + 164, 28), expected);
    assert_eq!(guest.get(ARG + 160, 4), [0xa7; 4]);
    assert_eq!(guest.get(ARG + 192, 8), [0xa7; 8]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Terminate,
            &[0]
        ),
        Ok(INVALID_CONTEXT)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Terminate,
            &[2]
        ),
        Ok(CONTEXT_NOT_FOUND)
    );
    census(&state, &budget);
}

#[test]
fn six_argument_create_observes_staged_id_string_overlaps_and_late_failures_do_not_publish() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX, 0]
        ),
        Ok(0)
    );
    assert_eq!((budget.usage().bytes_read, state.next_request), (0, 1000));
    guest.put(192, &[0xa7; 8]);
    assert!(matches!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, ARG, ARG + PAGE, ARG, 0, ARG + 192]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG + 192, 8), [0xa7; 8]);
    assert_eq!(
        (
            state.next_request,
            state.requests.len(),
            state.requests.capacity(),
            state.text_bytes
        ),
        (1000, 0, 0, 0)
    );
    guest.refuse = true;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, ARG, ARG + 32, ARG + 64, 0, ARG + 192]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(
        (
            state.next_request,
            state.requests.capacity(),
            state.retained_handle_count()
        ),
        (1000, 0, 0)
    );
    guest.refuse = false;
    guest.put(191, &[0xfe]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[
                (7 << 32) | 23,
                ARG + 192,
                ARG + 193,
                ARG + 191,
                u64::MAX,
                ARG + 192
            ]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(192), 1000);
    let request = &state.requests[0];
    assert_eq!(
        (
            &request.api_group[..],
            &request.path[..],
            &request.method[..]
        ),
        (&[0xe8, 3][..], &[3][..], &[0xfe, 0xe8, 3][..])
    );
    assert_eq!((request._user, request._content_parameter), (23, u64::MAX));
    assert_eq!(state.next_request, 1001);
    census(&state, &budget);
}

#[test]
fn send_is_offline_and_atomically_preserves_padding_optional_nul_aliases_and_request_state() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    request(&mut state, &mut budget, &mut guest);
    // A synthetic pre-existing response makes state rollback observable.
    state.requests[0].response = b"old response".to_vec();
    state.requests[0].read_offset = 3;
    state.text_bytes += state.requests[0].response.capacity();
    guest.put(256, &[0xa7; 40]);
    guest.put(264, &(ARG + PAGE).to_le_bytes());
    guest.put(272, &1u64.to_le_bytes());
    let before = guest.get(ARG + 256, 40);
    assert!(matches!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2SendRequest,
            &[1000, u64::MAX, u64::MAX, ARG + 256]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG + 256, 40), before);
    assert_eq!(
        (
            &state.requests[0].response[..],
            state.requests[0].read_offset
        ),
        (&b"old response"[..], 3)
    );
    for target in [ARG + 257, ARG + 282, ARG + 260, ARG + 300] {
        guest.put(256, &[0xa7; 64]);
        guest.put(264, &target.to_le_bytes());
        guest.put(272, &1u64.to_le_bytes());
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                NpWebApi2SendRequest,
                &[1000, u64::MAX, u64::MAX, ARG + 256]
            ),
            Ok(NOT_SIGNED_IN)
        );
        assert_eq!(guest.get(ARG + 256, 4), [0; 4]);
        assert_eq!(guest.get(ARG + 280, 8), [0; 8]);
        assert_eq!(guest.get(target, 1), [0]);
        if target != ARG + 260 {
            assert_eq!(guest.get(ARG + 260, 4), [0xa7; 4]);
        }
        assert!(state.requests[0].response.is_empty());
        assert_eq!(state.requests[0].read_offset, 0);
    }
    guest.put(264, &0u64.to_le_bytes());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2SendRequest,
            &[1000, u64::MAX, u64::MAX, ARG + 256]
        ),
        Ok(NOT_SIGNED_IN)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2SendRequest,
            &[1000, u64::MAX, u64::MAX, 0]
        ),
        Ok(NOT_SIGNED_IN)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2SendRequest,
            &[u64::MAX, u64::MAX, u64::MAX, u64::MAX]
        ),
        Ok(REQUEST_NOT_FOUND)
    );
    census(&state, &budget);
}

#[test]
fn send_refuses_unproved_descriptor_aliases_before_reading_or_writing_guest_fields() {
    struct Unproved<'a> {
        guest: &'a mut TestGuest,
        reads: core::cell::Cell<usize>,
        writes: usize,
    }
    impl GuestAccess for Unproved<'_> {
        fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), GuestError> {
            self.reads.set(self.reads.get() + 1);
            self.guest.read(address, bytes)
        }
        fn apply(
            &mut self,
            effect: crate::MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.writes += 1;
            self.guest.apply(effect, writes)
        }
    }
    let (mut state, mut budget, mut guest) = fixture(limits());
    request(&mut state, &mut budget, &mut guest);
    state.requests[0].read_offset = 7;
    let mut unproved = Unproved {
        guest: &mut guest,
        reads: core::cell::Cell::new(0),
        writes: 0,
    };
    assert_eq!(
        state.invoke(
            NpWebApi2SendRequest,
            CallArgs([1000, 0, 0, ARG + 256, 0, 0]),
            &mut unproved,
            &mut budget
        ),
        Err(Failure::Guest(GuestError::UnsupportedMapping))
    );
    assert_eq!(
        (
            unproved.reads.get(),
            unproved.writes,
            state.requests[0].read_offset
        ),
        (0, 0, 7)
    );
    assert_eq!(
        state.invoke(
            NpWebApi2SendRequest,
            CallArgs([123, 0, 0, u64::MAX, 0, 0]),
            &mut unproved,
            &mut budget
        ),
        Ok(REQUEST_NOT_FOUND)
    );
    assert_eq!((unproved.reads.get(), unproved.writes), (0, 0));
}

#[test]
fn headers_follow_two_exact_comparisons_truncate_nul_and_ignore_logging_only_adds() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2AddHttpRequestHeader,
            &[u64::MAX, u64::MAX, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(budget.usage().bytes_read, 0);
    request(&mut state, &mut budget, &mut guest);
    for (name, expected) in [
        (b"Content-Type\0".as_slice(), CONTENT_TYPE.len() + 1),
        (b"content-type\0".as_slice(), CONTENT_TYPE.len() + 1),
        (b"CONTENT-TYPE\0".as_slice(), 0),
    ] {
        guest.put(96, name);
        guest.put(200, &[0xa7; 48]);
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                NpWebApi2GetHttpResponseHeaderValueLength,
                &[1000, ARG + 96, ARG + 200]
            ),
            Ok(0)
        );
        assert_eq!(guest.u64(200), expected as u64);
        for size in [1, 5, u64::MAX] {
            assert_eq!(
                call(
                    &mut state,
                    &mut budget,
                    &mut guest,
                    NpWebApi2GetHttpResponseHeaderValue,
                    &[1000, ARG + 96, ARG + 216, size]
                ),
                Ok(0)
            );
            let length = if expected == 0 {
                0
            } else {
                (size - 1).min(CONTENT_TYPE.len() as u64) as usize
            };
            assert_eq!(guest.get(ARG + 216 + length as u64, 1), [0]);
            if length != 0 {
                assert_eq!(guest.get(ARG + 216, length), CONTENT_TYPE[..length]);
            }
        }
    }
    // Both strcmp calls stop at a first-byte mismatch; no trailing NUL exists.
    guest.put(PAGE - 1, b"X");
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2GetHttpResponseHeaderValueLength,
            &[1000, ARG + PAGE - 1, ARG + 200]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(200), 0);
    for (service, args, expected) in [
        (
            NpWebApi2GetHttpResponseHeaderValueLength,
            vec![0, 0, 0],
            INVALID_ARGUMENT,
        ),
        (
            NpWebApi2GetHttpResponseHeaderValueLength,
            vec![0, u64::MAX, ARG],
            REQUEST_NOT_FOUND,
        ),
        (
            NpWebApi2GetHttpResponseHeaderValue,
            vec![0, 1, 1, 0],
            INVALID_ARGUMENT,
        ),
        (
            NpWebApi2GetHttpResponseHeaderValue,
            vec![0, u64::MAX, ARG, 1],
            REQUEST_NOT_FOUND,
        ),
    ] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &args),
            Ok(expected)
        );
    }
}

#[test]
fn reads_and_deletes_preserve_source_null_precedence_stale_ids_and_full_request_width() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    request(&mut state, &mut budget, &mut guest);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2ReadData,
            &[0, 0, 1]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2ReadData,
            &[0, 1, 0]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2ReadData,
            &[(1 << 32) | 1000, u64::MAX, 1]
        ),
        Ok(REQUEST_NOT_FOUND)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2ReadData,
            &[1000, u64::MAX, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2DeleteRequest,
            &[1000]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2AbortRequest,
            &[1000]
        ),
        Ok(REQUEST_NOT_FOUND)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2DeleteRequest,
            &[1000]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, 0, 0, 0, 0, ARG + 128]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(128), 1001);
    census(&state, &budget);
}

#[test]
fn shared_handle_text_record_scratch_and_work_refusals_leave_ids_and_outputs_unchanged() {
    for policy in [
        Limits {
            max_handles: 0,
            ..limits()
        },
        Limits {
            max_work: 0,
            ..limits()
        },
        Limits {
            max_scratch_bytes: 1,
            max_name_bytes: 1,
            max_io_bytes: 1,
            ..limits()
        },
    ] {
        let (mut state, mut budget, mut guest) = fixture(policy);
        assert!(matches!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                NpWebApi2Initialize,
                &[0, 0]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(
            (
                state.next_context,
                state.contexts.capacity(),
                state.retained_handle_count()
            ),
            (1, 0, 0)
        );
        census(&state, &budget);
    }
    let (mut state, mut budget, mut guest) = fixture(Limits {
        max_io_bytes: 1,
        ..limits()
    });
    guest.put(128, &[0xa7; 8]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, ARG, 0, 0, 0, ARG + 128]
        ),
        Err(Failure::Limit("NpWebApi2 retained text bytes"))
    );
    assert_eq!(guest.get(ARG + 128, 8), [0xa7; 8]);
    assert_eq!(
        (
            state.next_request,
            state.retained_handle_count(),
            state.requests.capacity()
        ),
        (1000, 0, 0)
    );
    let (mut state, mut budget, mut guest) = fixture(limits());
    for counter in [
        &mut state.next_context,
        &mut state.next_push_handle,
        &mut state.next_user,
        &mut state.next_filter,
        &mut state.next_callback,
    ] {
        *counter = i32::MAX as u64;
    }
    for service in [
        NpWebApi2Initialize,
        NpWebApi2PushEventCreateHandle,
        NpWebApi2CreateUserContext,
        NpWebApi2PushEventCreateFilter,
        NpWebApi2PushEventRegisterCallback,
    ] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &[]),
            Ok(i32::MAX as u64)
        );
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &[]),
            Err(Failure::Limit("NpWebApi2 identifiers"))
        );
    }
    state.next_request = i64::MAX as u64;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, 0, 0, 0, 0, ARG + 128]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(128), i64::MAX as u64);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, 0, 0, 0, 0, ARG + 128]
        ),
        Err(Failure::Limit("NpWebApi2 identifiers"))
    );
    census(&state, &budget);
    let (mut state, mut budget, mut guest) = fixture(Limits {
        max_handles: 1,
        ..limits()
    });
    state.contexts.try_reserve_exact(2).unwrap();
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2Initialize,
            &[0, 0]
        ),
        Err(Failure::Limit("NpWebApi2 record capacity"))
    );
    assert_eq!((state.next_context, state.retained_handle_count()), (1, 0));
    census(&state, &budget);
    let (mut state, mut budget, mut guest) = fixture(Limits {
        max_name_bytes: usize::MAX,
        max_scratch_bytes: usize::MAX,
        ..limits()
    });
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            NpWebApi2CreateRequest,
            &[0, ARG, 0, 0, 0, ARG + 128]
        ),
        Err(Failure::Limit("work"))
    );
    assert_eq!(
        (
            state.next_request,
            state.retained_handle_count(),
            state.requests.capacity()
        ),
        (1000, 0, 0)
    );
}
