// SPDX-License-Identifier: GPL-2.0-only
//! Fixed elapsed inputs prove diagnostics without executing or timing a guest.
use super::{BuiltinRouteMetric, BuiltinRouteOutcome, ServiceProfile, StartupProfile};
fn fields(profile: &StartupProfile) -> String {
    let mut out = String::new();
    profile.append_builtin_route_fields(&mut out);
    out
}
#[test]
fn builtin_route_byte_bands_cover_zero_edges_and_full_width_requests_for_all_operations() {
    let mut profile = StartupProfile::default();
    for operation in 0..3 {
        for bytes in [
            0,
            1,
            64,
            65,
            4096,
            4097,
            65536,
            65537,
            1048576,
            1048577,
            u64::MAX,
        ] {
            profile.record_builtin_route(
                operation,
                Some(bytes),
                7,
                BuiltinRouteOutcome::Substituted,
            );
        }
    }
    for operation in 0..3 {
        assert_eq!(
            profile.builtin_routes[operation][0],
            BuiltinRouteMetric {
                count: 1,
                nanoseconds: 7
            }
        );
        for row in 1..6 {
            assert_eq!(
                profile.builtin_routes[operation][row],
                BuiltinRouteMetric {
                    count: 2,
                    nanoseconds: 14
                }
            );
        }
        assert_eq!(
            profile.builtin_routes[operation][6],
            BuiltinRouteMetric::default()
        );
        assert_eq!(
            profile.builtin_routes[operation][7],
            BuiltinRouteMetric::default()
        );
    }
    assert!(!profile.builtin_route_overflowed);
    let text = fields(&profile);
    for name in ["memcpy", "memmove", "memset"] {
        assert!(text.contains(&format!("libc_builtin_route_{name}=zero:1:7,1_64:2:14,65_4096:2:14,4097_65536:2:14,65537_1048576:2:14,over_1048576:2:14,fallback:0:0,refused:0:0")));
    }
}
#[test]
fn builtin_route_disabled_fallback_and_refusal_observations_never_claim_transfer_bytes() {
    let mut profile = StartupProfile::default();
    let before = profile.builtin_routes;
    for outcome in [
        BuiltinRouteOutcome::Substituted,
        BuiltinRouteOutcome::Fallback,
        BuiltinRouteOutcome::Refused,
    ] {
        profile.record_builtin_route(usize::MAX, None, u64::MAX, outcome);
    }
    assert_eq!(profile.builtin_routes, before);
    assert!(!profile.builtin_route_overflowed);
    profile.record_builtin_route(1, Some(u64::MAX), 11, BuiltinRouteOutcome::Fallback);
    profile.record_builtin_route(1, Some(0), 13, BuiltinRouteOutcome::Refused);
    assert_eq!(
        profile.builtin_routes[1][..6],
        [BuiltinRouteMetric::default(); 6]
    );
    assert_eq!(
        profile.builtin_routes[1][6],
        BuiltinRouteMetric {
            count: 1,
            nanoseconds: 11
        }
    );
    assert_eq!(
        profile.builtin_routes[1][7],
        BuiltinRouteMetric {
            count: 1,
            nanoseconds: 13
        }
    );
    assert_eq!(profile.service_ns, 0);
}
#[test]
fn builtin_route_count_duration_and_index_overflows_freeze_only_diagnostics_atomically() {
    for count_overflow in [false, true] {
        let mut profile = StartupProfile::default();
        profile.builtin_routes[0][0] = if count_overflow {
            BuiltinRouteMetric {
                count: u64::MAX,
                nanoseconds: 0,
            }
        } else {
            BuiltinRouteMetric {
                count: 1,
                nanoseconds: u64::MAX,
            }
        };
        let before = profile.builtin_routes;
        profile.record_builtin_route(0, Some(0), 1, BuiltinRouteOutcome::Substituted);
        assert!(profile.builtin_route_overflowed);
        assert_eq!(profile.builtin_routes, before);
        profile.record_builtin_route(2, Some(1024), 17, BuiltinRouteOutcome::Substituted);
        assert_eq!(profile.builtin_routes, before);
        assert_eq!(profile.service_ns, 0);
        assert!(fields(&profile).ends_with("libc_builtin_route_overflowed=true"));
    }
    let mut profile = StartupProfile::default();
    let before = profile.builtin_routes;
    profile.record_builtin_route(3, Some(1), 1, BuiltinRouteOutcome::Substituted);
    assert!(profile.builtin_route_overflowed);
    assert_eq!(profile.builtin_routes, before);
}
#[test]
fn builtin_route_success_time_is_a_subset_of_original_other_provider_time_with_refusals_separate() {
    let mut profile = StartupProfile::default();
    let mut work = 0;
    for (operation, bytes, nanos, outcome) in [
        (0, 32, 10, BuiltinRouteOutcome::Substituted),
        (2, 0, 15, BuiltinRouteOutcome::Substituted),
        (1, 100, 3, BuiltinRouteOutcome::Fallback),
        (0, 8, 5, BuiltinRouteOutcome::Refused),
    ] {
        let elapsed = profile.service_elapsed(nanos);
        profile.record_builtin_route(operation, Some(bytes), elapsed, outcome);
        match outcome {
            BuiltinRouteOutcome::Substituted => profile
                .services
                .record(None, false, elapsed, &mut work, u64::MAX)
                .unwrap(),
            _ => profile.services.record_unattributed(elapsed),
        }
    }
    let elapsed = profile.service_elapsed(13);
    profile
        .services
        .record(None, false, elapsed, &mut work, u64::MAX)
        .unwrap();
    let successful: u64 = profile
        .builtin_routes
        .iter()
        .flat_map(|rows| &rows[..6])
        .map(|m| m.nanoseconds)
        .sum();
    assert_eq!(successful, 25);
    assert_eq!(profile.services.other_ns, 38);
    assert_eq!(profile.services.unattributed_ns, 8);
    assert_eq!(profile.service_ns, 46);
    assert_eq!(work, 0);
    assert_eq!(profile.services.other, 3);
    assert_eq!(profile.services.overflow, 0);
}
#[test]
fn builtin_route_terminal_fields_fit_the_existing_record_bound_and_do_not_expand_progress() {
    const LABEL: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let profile = StartupProfile {
        builtin_routes: [[BuiltinRouteMetric {
            count: u64::MAX,
            nanoseconds: u64::MAX,
        }; 8]; 3],
        services: ServiceProfile {
            counts: [Some((LABEL, u64::MAX, u64::MAX, u64::MAX)); 64],
            ..Default::default()
        },
        ..Default::default()
    };
    let extra = fields(&profile);
    assert_eq!(extra.len(), 1342);
    assert!(extra.trim_start_matches('\t').split('\t').all(|part| {
        part.split_once('=').is_some_and(|(key, value)| {
            key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b','))
        })
    }));
    let mut progress = super::super::NativeSessionProgress {
        profile,
        ..Default::default()
    };
    // All scalar counters use their maximum decimal width. The unchanged stop
    // lookahead has a 4096-byte field cap; another 4096 conservatively reserves
    // optional handle/device/permission labels, and 1024 covers the worker envelope.
    progress.dispatch.blocks = u64::MAX;
    progress.dispatch.native_entries = u64::MAX;
    progress.dispatch.affine_loop_entries = u64::MAX;
    progress.dispatch.affine_loop_iterations = u64::MAX;
    progress.dispatch.register_loop_entries = u64::MAX;
    progress.dispatch.register_loop_iterations = u64::MAX;
    progress.dispatch.cache_publications = u64::MAX;
    progress.dispatch.cache_reuses = u64::MAX;
    progress.dispatch.cache_replacements = u64::MAX;
    progress.dispatch.cache_evicted_key_revisits = u64::MAX;
    progress.dispatch.native_instructions = u64::MAX;
    progress.dispatch.source_bytes = u64::MAX;
    progress.dispatch.emitted_bytes = u64::MAX;
    progress.dispatch.work_units = u64::MAX;
    progress.dispatch.memory_access_checks = u64::MAX;
    progress.dispatch.memory_bytes_checked = u64::MAX;
    progress.dispatch.segment_operations = u64::MAX;
    progress.dispatch.segment_bytes = u64::MAX;
    progress.dispatch.string_elements = u64::MAX;
    progress.dispatch.string_bytes = u64::MAX;
    progress.dispatch.vector_operations = u64::MAX;
    progress.dispatch.vector_bytes = u64::MAX;
    progress.dispatch.integer_operations = u64::MAX;
    progress.dispatch.integer_bytes = u64::MAX;
    progress.primary_native_instructions = u64::MAX;
    progress.service_stops = u64::MAX;
    progress.fetched_source_bytes = u64::MAX;
    progress.admission_work = u64::MAX;
    progress.initializer_calls = u64::MAX;
    progress.plan_cache_hits = u64::MAX;
    progress.plan_cache_misses = u64::MAX;
    progress.output_bytes = u64::MAX;
    progress.libc_builtin_bound_entries = u64::MAX;
    let terminal = progress.protocol_fields();
    assert!(terminal.len() + 4096 + 4096 + 1024 < 16 * 1024);
    let before_progress = progress.progress_fields();
    progress
        .profile
        .record_builtin_route(0, Some(0), 1, BuiltinRouteOutcome::Substituted);
    assert_eq!(progress.progress_fields(), before_progress);
    assert!(!before_progress.contains("libc_builtin_route_"));
}
