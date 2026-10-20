// SPDX-License-Identifier: GPL-2.0-only
// Original HTTP2 state fixtures; no host network or copied guest input.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest};
use crate::{Limits, Service::*};

fn fixture(limits: Limits) -> (Http2State, Budget, TestGuest) {
    let mut guest = TestGuest::new();
    guest.put(0, b"\xffM\0");
    guest.put(16, b"\xfeurl\0");
    guest.put(32, b"name\0");
    guest.put(48, b"value\0");
    (Http2State::default(), Budget::new(limits).unwrap(), guest)
}

fn limits() -> Limits {
    Limits {
        max_name_bytes: 16,
        max_io_bytes: 256,
        max_scratch_bytes: 1024,
        ..Default::default()
    }
}

fn call(
    state: &mut Http2State,
    budget: &mut Budget,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    state.invoke(service, CallArgs(args), guest, budget)
}

fn chain(state: &mut Http2State, budget: &mut Budget, guest: &mut TestGuest) {
    assert_eq!(call(state, budget, guest, Http2Init, &[0, 0, 0, 0]), Ok(1));
    assert_eq!(
        call(state, budget, guest, Http2CreateTemplate, &[1, 0, 0, 0]),
        Ok(1)
    );
    assert_eq!(
        call(
            state,
            budget,
            guest,
            Http2CreateRequestWithURL,
            &[1, ARG, ARG + 16, 0]
        ),
        Ok(1)
    );
}

fn snapshot(
    state: &Http2State,
    budget: &Budget,
) -> (usize, usize, usize, u64, u64, u64, usize, usize, usize) {
    (
        state.contexts.len(),
        state.templates.len(),
        state.requests.len(),
        state.next_context,
        state.next_template,
        state.next_request,
        state.text_bytes,
        state.header_count,
        budget.usage().handles_live,
    )
}

#[test]
fn independent_ids_prefer_request_options_without_inheritance_and_preserve_every_source_width() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2Init,
            &[u64::MAX, 1 << 32, u64::MAX, u64::MAX]
        ),
        Ok(1)
    );
    let context = &state.contexts[0];
    assert_eq!(
        (
            context._libnet_mem_id,
            context._libssl_ctx_id,
            context._pool_size,
            context._max_concurrent_requests
        ),
        (-1, 0, u64::MAX, -1)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[1 | (7 << 32), 0, u64::MAX, 1 << 32]
        ),
        Ok(1)
    );
    assert_eq!(state.templates[0].user_agent.capacity(), 0);
    assert_eq!(
        (
            state.templates[0]._http_ver,
            state.templates[0]._is_auto_proxy_conf
        ),
        (-1, 0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2SetAuthEnabled,
            &[1, 1]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[1, ARG, ARG + 16, 9]
        ),
        Ok(1)
    );
    assert_eq!(state.requests[0].options, Options::default());
    assert_eq!(
        (&state.requests[0].method[..], &state.requests[0].url[..]),
        (&b"\xffM"[..], &b"\xfeurl"[..])
    );
    let id = 1 | (11 << 32);
    for (service, value) in [
        (Http2SetAuthEnabled, 1 << 32),
        (Http2SetAutoRedirect, u64::MAX),
        (Http2SetInflateGZIPEnabled, 7),
        (Http2SslEnableOption, u64::MAX),
        (Http2SslDisableOption, (17 << 32) | 5),
        (Http2SetRecvTimeOut, (18 << 32) | 1),
        (Http2SetSendTimeOut, (19 << 32) | 2),
        (Http2SetConnectTimeOut, (20 << 32) | 3),
        (Http2SetConnectionWaitTimeOut, (21 << 32) | 4),
        (Http2SetResolveTimeOut, (22 << 32) | 5),
        (Http2SetTimeOut, (23 << 32) | 6),
        (Http2SetRequestContentLength, u64::MAX),
    ] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &[id, value]),
            Ok(0)
        );
    }
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2SetRedirectCallback,
            &[id, u64::MAX, u64::MAX - 1]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2SetSslCallback,
            &[id, 0, u64::MAX]
        ),
        Ok(0)
    );
    let options = state.requests[0].options;
    assert_eq!(
        options,
        Options {
            auth_enabled: false,
            auto_redirect: true,
            inflate_gzip: true,
            ssl_options: !5,
            recv_timeout_us: 1,
            send_timeout_us: 2,
            connect_timeout_us: 3,
            connection_wait_timeout_us: 4,
            resolve_timeout_us: 5,
            timeout_us: 6,
            redirect_callback: u64::MAX,
            redirect_user_arg: u64::MAX - 1,
            ssl_callback: 0,
            ssl_callback_arg: u64::MAX,
        }
    );
    assert_eq!(
        state.templates[0].options,
        Options {
            auth_enabled: true,
            ..Default::default()
        }
    );
    assert_eq!(state.requests[0].content_length, u64::MAX);
    for mode in [0, u64::MAX] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                Http2AddRequestHeader,
                &[id, ARG + 32, ARG + 48, mode]
            ),
            Ok(0)
        );
    }
    assert_eq!(state.requests[0].headers.len(), 2);
    assert_eq!(state.requests[0].headers[0], state.requests[0].headers[1]);
    assert_eq!(
        (
            &state.requests[0].headers[0].0[..],
            &state.requests[0].headers[0].1[..]
        ),
        (&b"name"[..], &b"value"[..])
    );
    assert_eq!(state.header_count, 2);
    assert_eq!(state.text_bytes, state.requests[0].text_bytes());
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn null_ordering_guest_refusals_and_work_limits_never_publish_partial_metadata_or_advance_ids() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    chain(&mut state, &mut budget, &mut guest);
    let before = snapshot(&state, &budget);
    let reads = budget.usage().bytes_read;
    for service in [Http2CreateRequestWithURL, Http2AddRequestHeader] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                service,
                &[999, 0, u64::MAX]
            ),
            Ok(NULL_POINTER)
        );
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                service,
                &[999, u64::MAX, 0]
            ),
            Ok(NULL_POINTER)
        );
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                service,
                &[999, u64::MAX, u64::MAX]
            ),
            Ok(INVALID_ID)
        );
    }
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[999, u64::MAX]
        ),
        Ok(INVALID_ID)
    );
    assert_eq!(budget.usage().bytes_read, reads);
    for service in [Http2CreateRequestWithURL, Http2AddRequestHeader] {
        // The first valid string is staged, then the second faults. Neither
        // strings, header count nor any new object identity may be published.
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                service,
                &[1, ARG, ARG + PAGE]
            ),
            Err(Failure::Guest(GuestError::Unmapped))
        );
        assert_eq!(snapshot(&state, &budget), before);
    }
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[1, ARG + PAGE]
        ),
        Err(Failure::Guest(GuestError::Unmapped))
    );
    guest.put(64, &[b'x'; 16]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[1, ARG + 64, ARG]
        ),
        Err(Failure::Limit("name bytes"))
    );
    assert_eq!(snapshot(&state, &budget), before);
    let work_limit = budget.limits.max_work;
    budget.limits.max_work = budget.usage().work;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2SetAutoRedirect,
            &[1, 1]
        ),
        Err(Failure::Limit("work"))
    );
    assert!(!state.requests[0].options.auto_redirect);
    assert_eq!(snapshot(&state, &budget), before);
    budget.limits.max_work = work_limit;
    budget.limits.max_bytes = budget.usage().bytes_read + budget.usage().bytes_written;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2AddRequestHeader,
            &[1, ARG, ARG]
        ),
        Err(Failure::Limit("guest bytes"))
    );
    assert_eq!(snapshot(&state, &budget), before);
}

#[test]
fn context_and_template_cascades_release_only_owned_lineage_without_reusing_any_process_id() {
    let (mut state, mut budget, mut guest) = fixture(limits());
    for expected in [1, 2] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, Http2Init, &[]),
            Ok(expected)
        );
    }
    for (parent, expected) in [(1, 1), (2, 2), (1, 3)] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                Http2CreateTemplate,
                &[parent, ARG + 32]
            ),
            Ok(expected)
        );
    }
    for (template, expected) in [(1, 1), (3, 2), (2, 3)] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                Http2CreateRequestWithURL,
                &[template, ARG, ARG + 16]
            ),
            Ok(expected)
        );
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                Http2AddRequestHeader,
                &[expected, ARG + 32, ARG + 48]
            ),
            Ok(0)
        );
    }
    assert_eq!(state.retained_handle_count(), 8);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2DeleteTemplate,
            &[1]
        ),
        Ok(0)
    );
    assert_eq!(state.retained_handle_count(), 6);
    assert_eq!(
        state.requests.iter().map(|r| r.id).collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[1, 0]
        ),
        Ok(4)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[2, ARG, ARG + 16]
        ),
        Ok(4)
    );
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Term, &[1]),
        Ok(0)
    );
    assert_eq!(state.retained_handle_count(), 4);
    assert_eq!(state.header_count, 1);
    assert_eq!(
        state.templates.iter().map(|t| t.id).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        state.requests.iter().map(|r| r.id).collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2DeleteRequest,
            &[4]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2DeleteRequest,
            &[4]
        ),
        Ok(INVALID_ID)
    );
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Term, &[2]),
        Ok(0)
    );
    assert_eq!(
        (
            state.retained_handle_count(),
            state.text_bytes,
            state.header_count,
            budget.usage().handles_live
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(
        (state.next_context, state.next_template, state.next_request),
        (3, 5, 5)
    );
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Init, &[]),
        Ok(3)
    );
    let (mut other, mut other_budget, mut other_guest) = fixture(limits());
    assert_eq!(
        call(
            &mut other,
            &mut other_budget,
            &mut other_guest,
            Http2Init,
            &[]
        ),
        Ok(1)
    );
    assert_eq!(state.contexts[0].id, 3);
}

#[test]
fn shared_handles_aggregate_text_and_headers_and_i32_identity_ceiling_are_finite_and_atomic() {
    let (mut state, mut budget, mut guest) = fixture(Limits {
        max_handles: 2,
        ..limits()
    });
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Init, &[]),
        Ok(1)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[1]
        ),
        Ok(1)
    );
    let before = snapshot(&state, &budget);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[1, ARG, ARG + 16]
        ),
        Err(Failure::Limit("handles"))
    );
    assert_eq!(snapshot(&state, &budget), before);
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Term, &[1]),
        Ok(0)
    );
    assert_eq!(budget.usage().handles_live, 0);

    let (mut state, mut budget, mut guest) = fixture(Limits {
        max_io_bytes: 4,
        ..limits()
    });
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Init, &[]),
        Ok(1)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[1, ARG + 32]
        ),
        Ok(1)
    );
    assert_eq!(state.text_bytes, state.templates[0].user_agent.capacity());
    let before = snapshot(&state, &budget);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[1, ARG, ARG + 16]
        ),
        Err(Failure::Limit("HTTP2 retained text bytes"))
    );
    assert_eq!(snapshot(&state, &budget), before);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2DeleteTemplate,
            &[1]
        ),
        Ok(0)
    );
    assert_eq!(state.text_bytes, 0);
    // Empty strings retain zero capacity even with a large scan allowance.
    guest.put(64, &[0]);
    budget.limits.max_io_bytes = 0;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[1, ARG + 64]
        ),
        Ok(2)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[2, ARG + 64, ARG + 64]
        ),
        Ok(1)
    );
    assert_eq!(state.text_bytes, 0);

    let (mut state, mut budget, mut guest) = fixture(Limits {
        max_handles: 3,
        ..limits()
    });
    chain(&mut state, &mut budget, &mut guest);
    for _ in 0..3 {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                Http2AddRequestHeader,
                &[1, ARG + 32, ARG + 48]
            ),
            Ok(0)
        );
    }
    let before = snapshot(&state, &budget);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2AddRequestHeader,
            &[1, ARG + 32, ARG + 48]
        ),
        Err(Failure::Limit("HTTP2 headers"))
    );
    assert_eq!(snapshot(&state, &budget), before);
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Term, &[1]),
        Ok(0)
    );
    assert_eq!(
        (
            state.header_count,
            state.text_bytes,
            budget.usage().handles_live
        ),
        (0, 0, 0)
    );

    let (mut state, mut budget, mut guest) = fixture(limits());
    let maximum = i32::MAX as u64;
    state.next_context = maximum;
    state.next_template = maximum;
    state.next_request = maximum;
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, Http2Init, &[]),
        Ok(maximum)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateTemplate,
            &[maximum]
        ),
        Ok(maximum)
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            Http2CreateRequestWithURL,
            &[maximum, ARG, ARG + 16]
        ),
        Ok(maximum)
    );
    let before = snapshot(&state, &budget);
    for (service, args) in [
        (Http2Init, [0, 0, 0]),
        (Http2CreateTemplate, [maximum, 0, 0]),
        (Http2CreateRequestWithURL, [maximum, ARG, ARG + 16]),
    ] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &args),
            Err(Failure::Limit("HTTP2 identifiers"))
        );
        assert_eq!(snapshot(&state, &budget), before);
    }
    assert_eq!(
        (state.next_context, state.next_template, state.next_request),
        (maximum + 1, maximum + 1, maximum + 1)
    );
}
