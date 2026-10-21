// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use std::time::Duration;

#[test]
fn affine_pairs_observe_each_priority_quantum_and_pending_switch_without_mutation() {
    let mut scheduling = Scheduling::new(None, 1).unwrap();
    scheduling.last_switch = 7;
    for (class, span) in [
        (ThreadPriorityClass::High, 4096),
        (ThreadPriorityClass::Normal, 2048),
        (ThreadPriorityClass::Low, 1024),
    ] {
        assert_eq!(scheduling.trace_turn_remaining(7 + span - 2, class), 2);
        assert_eq!(scheduling.trace_turn_remaining(7 + span - 1, class), 1);
        assert_eq!(scheduling.trace_turn_remaining(7 + span, class), 0);
        assert_eq!(scheduling.trace_turn_remaining(6, class), 0);
        assert_eq!(scheduling.last_switch, 7);
        scheduling.force_switch = true;
        assert_eq!(scheduling.trace_turn_remaining(7, class), 0);
        assert!(scheduling.force_switch);
        scheduling.force_switch = false;
        scheduling.threads[0].returned = true;
        assert_eq!(scheduling.trace_turn_remaining(7, class), 0);
        scheduling.threads[0].returned = false;
    }
}

#[test]
fn finite_consumer_wake_precedes_deadlock_without_satisfying_or_charging_guest_waits() {
    // Without a consumer, an indefinite guest wait retains the original
    // no-deadline outcome. An actual consumer deadline allows only a bounded
    // scheduling slice; it does not manufacture a returned guest syscall.
    assert_eq!(idle_slice(None, None), None);
    let wake = super::super::frame_bridge::ExternalWake {
        delay: Duration::from_millis(5),
        maximum_idle_nanoseconds: 2_000_000,
    };
    assert_eq!(
        idle_slice(None, Some(wake)),
        Some((Duration::from_millis(1), Some(2_000_000)))
    );
    assert_eq!(
        idle_slice(Some(Duration::from_secs(1)), Some(wake)),
        Some((Duration::from_millis(1), Some(2_000_000)))
    );
    // An earlier guest timer continues to consume the original guest budget.
    assert_eq!(
        idle_slice(Some(Duration::from_micros(17)), Some(wake)),
        Some((Duration::from_micros(17), None))
    );
    let mut external = 0;
    let guest = 0;
    for _ in 0..2 {
        let (delay, cap) = idle_slice(None, Some(wake)).unwrap();
        charge(
            &mut external,
            delay.as_nanos() as u64,
            cap.unwrap(),
            "frame idle waits",
        )
        .unwrap();
    }
    assert_eq!((external, guest), (2_000_000, 0));
    assert!(matches!(
        charge(&mut external, 1, 2_000_000, "frame idle waits"),
        Err(NativeSessionError::Budget("frame idle waits"))
    ));
    assert_eq!(external, 2_000_000);
}
