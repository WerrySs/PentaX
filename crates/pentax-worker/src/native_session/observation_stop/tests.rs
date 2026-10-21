// SPDX-License-Identifier: GPL-2.0-only
//! Deterministic host clocks, with no sleeping, guest execution or quota changes.
use super::*;
use std::cell::Cell;

#[test]
fn disabled_observation_and_invalid_profiles_do_not_read_a_clock() {
    for interactive in [false, true] {
        let mut stop = ObservationStop::new(None, interactive).unwrap();
        stop.start(|| panic!("disabled start read the clock"))
            .unwrap();
        for blocks in [0, 1024, u64::MAX] {
            stop.check(blocks, true, || panic!("disabled check read the clock"))
                .unwrap();
            assert_eq!(stop.maximum_span(blocks), 2);
        }
    }
    assert!(
        super::super::NativeSessionLimits::default()
            .stop_after_seconds
            .is_none()
    );
    for seconds in [0, 3601, u32::MAX] {
        assert!(ObservationStop::new(Some(seconds), true).is_err());
    }
    assert!(ObservationStop::new(Some(1), false).is_err());
}

#[test]
fn one_deadline_survives_epoch_rebinding_and_checks_both_edges_without_reset() {
    let start = Instant::now();
    let calls = Cell::new(0);
    let mut stop = ObservationStop::new(Some(1), true).unwrap();
    stop.start(|| {
        calls.set(calls.get() + 1);
        start
    })
    .unwrap();
    // The same state is reused when execute_epoch reattaches the next source.
    stop.start(|| panic!("a new epoch restarted the clock"))
        .unwrap();
    assert_eq!(calls.get(), 1);
    stop.check(0, false, || start).unwrap();
    stop.check(1024, false, || start + Duration::from_nanos(999_999_999))
        .unwrap();
    assert!(matches!(
        stop.check(1024, true, || start + Duration::from_secs(1)),
        Err(NativeSessionError::ObservationStopped)
    ));
    assert_eq!(stop.deadline, Some(start + Duration::from_secs(1)));
    stop.start(|| panic!("expired deadline was reset")).unwrap();
    assert!(matches!(
        stop.check(2048, false, || start + Duration::from_secs(20)),
        Err(NativeSessionError::ObservationStopped)
    ));
}

#[test]
fn cadence_bounds_real_trace_retirement_and_idle_retry_checks_are_forced() {
    let start = Instant::now();
    let mut stop = ObservationStop::new(Some(1), true).unwrap();
    stop.start(|| start).unwrap();
    stop.check(0, false, || start).unwrap();
    assert_eq!(stop.maximum_span(1022), 2);
    assert_eq!(stop.maximum_span(1023), 1);
    assert_eq!(stop.maximum_span(1024), 1);
    stop.check(1023, false, || {
        panic!("cadence performed an early clock read")
    })
    .unwrap();
    stop.check(1024, false, || start + Duration::from_millis(1))
        .unwrap();
    assert_eq!(stop.maximum_span(1024), 2);
    // No logical block must retire for an idle/retry observation to expire.
    assert!(matches!(
        stop.check(1024, true, || start + Duration::from_secs(1)),
        Err(NativeSessionError::ObservationStopped)
    ));
    let mut maximum = ObservationStop::new(Some(3600), true).unwrap();
    maximum.start(|| start).unwrap();
    maximum.check(u64::MAX - 1, false, || start).unwrap();
    assert_eq!(maximum.maximum_span(u64::MAX), 2); // Existing trace guard refuses counter overflow.
    assert_eq!(maximum.maximum_span(0), 1); // A counter reversal never grants a pair.
}
