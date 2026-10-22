// SPDX-License-Identifier: GPL-2.0-only

use super::{ServiceProfile, StartupProfile};

fn field<'a>(fields: &'a str, key: &str) -> &'a str {
    fields
        .split('\t')
        .filter_map(|part| part.split_once('='))
        .find_map(|(name, value)| (name == key).then_some(value))
        .unwrap()
}

#[test]
fn measured_totals_rank_duration_independently_from_calls_and_nonzero_returns() {
    let mut profile = StartupProfile::default();
    let mut work = 0;
    for (name, calls, nanos, nonzero) in [
        ("PthreadMutexLock", 9, 1, false),
        ("PthreadMutexUnlock", 8, 2, false),
        ("pthread_mutex_lock", 7, 3, false),
        ("pthread_mutex_unlock", 6, 4, false),
        ("Open", 1, 500, true),
    ] {
        for _ in 0..calls {
            let elapsed = profile.service_elapsed(nanos);
            profile
                .services
                .record(Some(name), nonzero, elapsed, &mut work, u64::MAX)
                .unwrap();
        }
    }
    let fields = profile.protocol_fields();
    assert_eq!(profile.service_ns, 570);
    assert_eq!(
        field(&fields, "service_top"),
        "PthreadMutexLock:9,PthreadMutexUnlock:8,pthread_mutex_lock:7,pthread_mutex_unlock:6"
    );
    assert_eq!(field(&fields, "service_nonzero_top"), "Open:1");
    assert_eq!(
        field(&fields, "service_time_top"),
        "Open:500,pthread_mutex_unlock:24,pthread_mutex_lock:21,PthreadMutexUnlock:16"
    );
    assert_eq!(
        profile
            .services
            .counts
            .iter()
            .flatten()
            .map(|row| row.3)
            .sum::<u64>(),
        profile.service_ns
    );
}

#[test]
fn fixed_capacity_preserves_distinct_other_and_overflow_time_without_more_slots() {
    const LABELS: [&str; 64] = [
        "Label00", "Label01", "Label02", "Label03", "Label04", "Label05", "Label06", "Label07",
        "Label08", "Label09", "Label10", "Label11", "Label12", "Label13", "Label14", "Label15",
        "Label16", "Label17", "Label18", "Label19", "Label20", "Label21", "Label22", "Label23",
        "Label24", "Label25", "Label26", "Label27", "Label28", "Label29", "Label30", "Label31",
        "Label32", "Label33", "Label34", "Label35", "Label36", "Label37", "Label38", "Label39",
        "Label40", "Label41", "Label42", "Label43", "Label44", "Label45", "Label46", "Label47",
        "Label48", "Label49", "Label50", "Label51", "Label52", "Label53", "Label54", "Label55",
        "Label56", "Label57", "Label58", "Label59", "Label60", "Label61", "Label62", "Label63",
    ];
    let mut profile = StartupProfile::default();
    let mut work = 0;
    for label in LABELS {
        let elapsed = profile.service_elapsed(1);
        profile
            .services
            .record(Some(label), false, elapsed, &mut work, u64::MAX)
            .unwrap();
    }
    let elapsed = profile.service_elapsed(15);
    profile
        .services
        .record(Some("Unretained"), true, elapsed, &mut work, u64::MAX)
        .unwrap();
    let before_other = work;
    let elapsed = profile.service_elapsed(11);
    profile
        .services
        .record(None, false, elapsed, &mut work, u64::MAX)
        .unwrap();
    assert_eq!(work, before_other);
    assert_eq!(profile.services.counts.len(), 64);
    assert_eq!(profile.services.counts.iter().flatten().count(), 64);
    assert!(
        !profile
            .services
            .counts
            .iter()
            .flatten()
            .any(|row| row.0 == "Unretained")
    );
    let fields = profile.protocol_fields();
    assert_eq!(profile.service_ns, 90);
    assert_eq!(field(&fields, "service_other"), "1");
    assert_eq!(field(&fields, "service_overflow"), "1");
    assert_eq!(field(&fields, "service_other_ns"), "11");
    assert_eq!(field(&fields, "service_overflow_ns"), "15");
    assert_eq!(field(&fields, "service_unattributed_ns"), "0");
    assert_eq!(
        profile
            .services
            .counts
            .iter()
            .flatten()
            .map(|row| row.3)
            .sum::<u64>()
            + profile.services.other_ns
            + profile.services.overflow_ns,
        profile.service_ns
    );
}

#[test]
fn failed_and_unadmitted_measurements_never_borrow_the_previous_successful_label() {
    let mut profile = StartupProfile::default();
    let mut work = 0;
    let elapsed = profile.service_elapsed(7);
    profile
        .services
        .record(Some("Open"), false, elapsed, &mut work, u64::MAX)
        .unwrap();
    let old_counts = profile.services.counts;
    let old_last = profile.services.last;
    // A failed service or a libc fallback has no successful catalogue label.
    let elapsed = profile.service_elapsed(100);
    profile.services.record_unattributed(elapsed);
    assert_eq!(profile.services.counts, old_counts);
    assert_eq!(profile.services.last, old_last);
    let elapsed = profile.service_elapsed(23);
    let limit = work;
    assert!(
        profile
            .services
            .record(Some("Close"), false, elapsed, &mut work, limit)
            .is_err()
    );
    assert_eq!(profile.services.counts, old_counts);
    assert_eq!(profile.services.other, 0);
    assert_eq!(profile.services.overflow, 0);
    assert_eq!(profile.service_ns, 130);
    let fields = profile.protocol_fields();
    assert_eq!(field(&fields, "service_time_top"), "Open:7");
    assert_eq!(field(&fields, "service_unattributed_ns"), "123");
    assert_eq!(field(&fields, "service_other_ns"), "0");
    assert_eq!(field(&fields, "service_overflow_ns"), "0");
}

#[test]
fn missing_timer_and_saturating_durations_keep_existing_default_and_counter_limits() {
    let mut profile = StartupProfile::default();
    let mut work = 0;
    assert_eq!(profile.service_finished(None), 0);
    assert_eq!(profile.service_ns, 0);
    assert_eq!(
        field(&profile.protocol_fields(), "service_time_top"),
        "none"
    );
    for _ in 0..2 {
        let elapsed = profile.service_elapsed(u128::MAX);
        assert_eq!(elapsed, u64::MAX);
        profile
            .services
            .record(Some("Open"), true, elapsed, &mut work, u64::MAX)
            .unwrap();
        profile
            .services
            .record(None, false, elapsed, &mut work, u64::MAX)
            .unwrap();
        profile.services.record_unattributed(elapsed);
    }
    assert_eq!(profile.service_ns, u64::MAX);
    assert_eq!(profile.services.counts[0], Some(("Open", 2, 2, u64::MAX)));
    assert_eq!(profile.services.other_ns, u64::MAX);
    assert_eq!(profile.services.unattributed_ns, u64::MAX);
    profile.services.overflow_ns = u64::MAX - 1;
    profile.services.counts = [Some(("Occupied", 1, 0, 0)); 64];
    profile
        .services
        .record(Some("Other"), false, 9, &mut work, u64::MAX)
        .unwrap();
    assert_eq!(profile.services.overflow_ns, u64::MAX);
}

#[test]
fn duration_observations_reuse_exact_record_admission_and_bounded_protocol_storage() {
    let mut zero = ServiceProfile::default();
    let mut timed = ServiceProfile::default();
    let mut zero_work = 0;
    let mut timed_work = 0;
    for label in [Some("Open"), Some("Close"), None, Some("Open")] {
        zero.record(label, false, 0, &mut zero_work, u64::MAX)
            .unwrap();
        timed
            .record(label, false, u64::MAX, &mut timed_work, u64::MAX)
            .unwrap();
        assert_eq!(zero_work, timed_work);
    }
    assert_eq!(zero_work, 40);
    assert_eq!(zero.last, timed.last);
    for (a, b) in zero
        .counts
        .iter()
        .flatten()
        .zip(timed.counts.iter().flatten())
    {
        assert_eq!((a.0, a.1, a.2), (b.0, b.1, b.2));
    }
    // The existing CLI record cap is 16 KiB. Four maximum 128-byte public labels,
    // three rankings and all timing counters fit without changing that cap.
    const LABEL: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    timed.counts = [Some((LABEL, u64::MAX, u64::MAX, u64::MAX)); 64];
    let fields = timed.protocol_fields();
    assert!(fields.len() < 2048);
    assert!(
        fields
            .split('\t')
            .all(|part| part.split_once('=').is_some_and(|(_, value)| value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b','))))
    );
    let progress = super::super::NativeSessionProgress {
        profile: StartupProfile {
            services: timed,
            ..StartupProfile::default()
        },
        ..super::super::NativeSessionProgress::default()
    };
    assert!(progress.protocol_fields().len() + 256 < 16 * 1024);
}
