// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_condition_wait_parks_child_and_reacquires_mutex_before_both_wakeups() {
    condition_wakeups(false);
}

#[test]
fn native_condition_observations_distinguish_a_real_barging_acquisition() {
    condition_wakeups(true);
}

fn condition_wakeups(barge: bool) {
    let imports = [
        "6UgtwV+0zb4",
        "9UK1vLZQft4",
        "tn3VlD0hG60",
        "WKAXJ4XBPQ4",
        "kDh-NfxgMtE",
        "JGgj7Uvrl+A",
    ];
    let mut p = Program::new();
    for offset in [0, 8, 16, 24] {
        p.set(offset, 0);
    }
    p.ptr(7, 16);
    p.reg(6, 0);
    p.path(2, 0xc00);
    p.ptr(1, 0);
    p.reg(8, 0);
    p.call(0);
    p.expect(0);
    p.word(24, 1); // Child has released its mutex and parked in CondWait.
    p.ptr(7, 8);
    p.call(1);
    p.expect(0);
    p.ptr(7, 0);
    p.call(4);
    p.expect(0);
    // Cross a scheduler quantum while holding the signaled waiter's mutex.
    // It must stay parked rather than returning early with invented ownership.
    p.reg(12, 3000);
    let loop_start = p.code.len();
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]);
    let offset = loop_start as i32 - (p.code.len() as i32 + 4);
    p.code.extend(offset.to_le_bytes());
    p.word(24, 1);
    p.ptr(7, 8);
    p.call(2);
    p.expect(0);
    if barge {
        // A short free interval precedes the next ready-queue scan. This is
        // one real reacquisition by the notifier, not a starvation claim.
        p.ptr(7, 8);
        p.call(1);
        p.expect(0);
        p.word(24, 1);
        p.ptr(7, 8);
        p.call(2);
        p.expect(0);
    }
    p.load(0, 24);
    p.code.extend([0x48, 0x83, 0xf8, 2, 0x75, 0xf5]);
    p.ptr(7, 8);
    p.call(1);
    p.expect(0);
    p.ptr(7, 0);
    p.call(5); // A new wait requires a new notification.
    p.expect(0);
    p.word(24, 2);
    p.ptr(7, 8);
    p.call(2);
    p.expect(0);
    p.load(0, 24);
    p.code.extend([0x48, 0x83, 0xf8, 3, 0x75, 0xf5]);
    p.ptr(7, 8);
    p.call(1);
    p.expect(0);
    p.ptr(7, 8);
    p.call(2);
    p.expect(0);
    let (mut code, calls) = p.finish(None);
    assert!(code.len() <= 0x400);
    code.resize(0x400, 0xcc);
    let mut child = vec![0x53, 0x48, 0x89, 0xfb];
    child.extend([0x48, 0x8d, 0x7b, 8]);
    rip(&mut child, 0xc00, &[0xff, 0x15], 0xf08);
    for stage in [1_u8, 2] {
        child.extend([0x48, 0xc7, 0x43, 24, stage, 0, 0, 0]);
        child.extend([0x48, 0x89, 0xdf, 0x48, 0x8d, 0x73, 8]);
        rip(&mut child, 0xc00, &[0xff, 0x15], 0xf18);
    }
    child.extend([0x48, 0xc7, 0x43, 24, 3, 0, 0, 0]);
    child.extend([0x48, 0x8d, 0x7b, 8]);
    rip(&mut child, 0xc00, &[0xff, 0x15], 0xf10);
    child.extend([0xb8, 73, 0, 0, 0, 0x5b, 0xc3]);
    code.extend(child);
    for library in [None, Some("Posix"), Some("libScePosix")] {
        let main = if let Some(library) = library {
            let scoped: Vec<_> = imports
                .iter()
                .enumerate()
                .map(|(index, nid)| {
                    if index == 3 {
                        ("Op8TBGY5KHg", true)
                    } else {
                        (*nid, false)
                    }
                })
                .collect();
            posix::scoped_image_with_library(&code, &scoped, 1, library)
        } else {
            image(&code, &imports)
        };
        let (status, fields, text) = probe(&main);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], (calls + 4).to_string());
        assert_eq!(fields["phase"], "main-entry");
        assert_eq!(fields["pthread_mutex_waits"], "0");
        assert_eq!(fields["pthread_condition_waits"], "0");
        assert_eq!(fields["pthread_notified_condition_waits"], "0");
        assert_eq!(
            fields["pthread_eligible_mutex_releases"],
            if barge { "3" } else { "2" }
        );
        assert_eq!(
            fields["pthread_barging_mutex_acquisitions"],
            if barge { "1" } else { "0" }
        );
        assert_eq!(fields["pthread_wait_observations_overflowed"], "false");
    }
}

#[test]
fn native_condition_wait_with_no_runnable_notifier_reports_deadlock_without_returning() {
    let mut p = Program::new();
    p.set(0, 0);
    p.set(8, 0);
    p.ptr(7, 8);
    p.call(0);
    p.expect(0);
    p.ptr(7, 0);
    p.ptr(6, 8);
    p.call(1);
    let (code, _) = p.finish(None);
    let main = image(&code, &["9UK1vLZQft4", "WKAXJ4XBPQ4"]);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "thread-deadlock");
    assert_eq!(fields["service_stops"], "2");
    assert!(!fields.contains_key("value"));
    assert_eq!(fields["pthread_condition_waits"], "1");
    assert_eq!(fields["pthread_notified_condition_waits"], "0");
    assert_eq!(fields["pthread_barging_mutex_acquisitions"], "0");
    // A still-running child can cease to be a notifier by returning while its
    // parent is parked. That is a real guest deadlock, not an executor invariant.
    let mut p = Program::new();
    for offset in [0, 8, 16] {
        p.set(offset, 0);
    }
    p.ptr(7, 16);
    p.reg(6, 0);
    p.path(2, 0xc00);
    p.reg(1, 0);
    p.reg(8, 0);
    p.call(2);
    p.expect(0);
    p.ptr(7, 8);
    p.call(0);
    p.expect(0);
    p.ptr(7, 0);
    p.ptr(6, 8);
    p.call(1);
    let (mut code, _) = p.finish(None);
    assert!(code.len() < 0x400);
    code.resize(0x400, 0xcc);
    code.extend([0xb9, 0xb8, 0x0b, 0, 0, 0xff, 0xc9, 0x75, 0xfc, 0xc3]);
    let main = image(&code, &["9UK1vLZQft4", "WKAXJ4XBPQ4", "6UgtwV+0zb4"]);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "thread-deadlock");
    assert!(!fields.contains_key("value"));
}

#[test]
fn native_condition_lazy_broadcast_and_named_lifecycle_use_real_owned_prefixes() {
    let mut p = Program::new();
    p.set(0, 0);
    p.ptr(7, 0);
    p.call(0); // POSIX broadcast lazily creates the zero slot.
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x83, 0x8b, 0xfc, 0, 0, 0, 0x42]); // OR [r11+252],0x42
    p.ptr(7, 0);
    p.call(1); // Signal existing condition.
    p.expect(0);
    p.load(11, 0); // R11 is caller-clobbered across the service call.
    p.code.extend([0x41, 0x8b, 0x83, 0xfc, 0, 0, 0]);
    p.expect(0x42);
    p.ptr(7, 0);
    p.call(2); // Destroy and clear slot.
    p.expect(0);
    p.word(0, 0);
    p.ptr(7, 8);
    p.set(8, 0);
    p.call(3); // Attribute init.
    p.expect(0);
    p.ptr(7, 8);
    p.reg(6, 4);
    p.call(4); // Retain monotonic clock.
    p.expect(0);
    p.ptr(7, 0);
    p.ptr(6, 8);
    p.reg(2, 0);
    p.call(5); // Explicit named kernel init, null name.
    p.expect(0);
    p.ptr(7, 8);
    p.call(6); // Destroy copied attribute.
    p.expect(0);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    p.ptr(7, 0);
    p.call(2);
    p.expect(0);
    p.reg(7, 0);
    p.call(0);
    p.expect(22); // Positive POSIX error, not a kernel int32.
    let (code, calls) = p.finish(None);
    let main = image(
        &code,
        &[
            "mkx2fVhNMsg",
            "kDh-NfxgMtE",
            "g+PZd2hiacg",
            "m5-2bsNfv7s",
            "EjllaAqAPZo",
            "2Tb92quprl0",
            "waPcxYiR3WA",
        ],
    );
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
