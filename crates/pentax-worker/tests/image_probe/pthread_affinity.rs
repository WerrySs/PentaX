// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_affinity_round_trip_and_attribute_snapshot_share_the_actual_thread_mask() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.load(7, 0);
    p.ptr(6, 24);
    p.call(2);
    p.expect(0);
    p.word(24, 0x7f);
    let mask = 0xfedc_ba98_7654_3210;
    p.load(7, 0);
    p.reg(6, mask);
    p.call(1);
    p.expect(0);
    p.load(7, 0);
    p.ptr(6, 24);
    p.call(2);
    p.expect(0);
    p.word(24, mask);
    p.set(8, 0);
    p.ptr(7, 8);
    p.call(3);
    p.expect(0);
    p.load(7, 0);
    p.ptr(6, 8);
    p.call(4);
    p.expect(0);
    p.ptr(7, 8);
    p.ptr(6, 24);
    p.call(5);
    p.expect(0);
    p.word(24, mask);
    for (slot, handle, output, expected) in [(1, 0, 0, 0x8002_0003), (2, 0, 0, 0x8002_0003)] {
        p.reg(7, handle);
        p.reg(6, output);
        p.call(slot);
        p.expect(expected);
    }
    p.load(7, 0);
    p.reg(6, 0);
    p.call(2);
    p.expect(0x8002_0016);
    p.load(7, 0);
    p.reg(6, 0);
    p.call(1);
    p.expect(0);
    p.load(7, 0);
    p.ptr(6, 24);
    p.call(2);
    p.expect(0);
    p.word(24, 0);
    p.ptr(7, 8);
    p.call(6);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "aI+OeCz8xrQ",
            "bt3CTBKmGyI",
            "rcrVFJsQWRY",
            "nsYoNRywwNg",
            "x1X76arYMxU",
            "8+s5BzZjxSg",
            "62KCwEMmzcM",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_created_thread_inherits_a_snapshot_and_observes_targeted_affinity_updates() {
    let initial = 0x8000_0000_0000_0040;
    let updated = 0xfedc_ba98_7654_3210;
    let mut p = Program::new();
    p.set(8, 0);
    p.set(16, 0);
    p.set(32, 0);
    p.ptr(7, 8);
    p.call(3);
    p.expect(0);
    p.ptr(7, 8);
    p.reg(6, initial);
    p.call(4);
    p.expect(0);
    p.ptr(7, 16);
    p.ptr(6, 8);
    p.path(2, 0xc00);
    p.ptr(1, 0);
    p.reg(8, 0);
    p.call(6);
    p.expect(0);
    p.word(32, 1);
    p.word(48, initial);
    p.load(0, 16);
    p.load(10, 40);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.ptr(7, 8);
    p.reg(6, 2);
    p.call(4);
    p.expect(0);
    p.ptr(7, 8);
    p.call(5);
    p.expect(0);
    p.load(7, 16);
    p.ptr(6, 64);
    p.call(2);
    p.expect(0);
    p.word(64, initial);
    p.load(7, 16);
    p.reg(6, updated);
    p.call(1);
    p.expect(0);
    p.set(32, 2);
    p.load(0, 32);
    p.code.extend([0x48, 0x83, 0xf8, 3, 0x75, 0xf5]);
    p.word(56, updated);
    let (mut code, _) = p.finish(None);
    assert!(code.len() < 0x400);
    code.resize(0x400, 0xcc);
    let mut child = vec![0x53, 0x48, 0x89, 0xfb];
    rip(&mut child, 0xc00, &[0xff, 0x15], 0xf00);
    child.extend([0x48, 0x89, 0x43, 40, 0x48, 0x89, 0xc7, 0x48, 0x8d, 0x73, 48]);
    rip(&mut child, 0xc00, &[0xff, 0x15], 0xf10);
    child.extend([0x48, 0xc7, 0x43, 32, 1, 0, 0, 0]);
    child.extend([0x48, 0x83, 0x7b, 32, 2, 0x75, 0xf9]);
    child.extend([0x48, 0x8b, 0x7b, 40, 0x48, 0x8d, 0x73, 56]);
    rip(&mut child, 0xc00, &[0xff, 0x15], 0xf10);
    child.extend([0x48, 0xc7, 0x43, 32, 3, 0, 0, 0, 0x5b, 0xc3]);
    code.extend(child);
    let source = image(
        &code,
        &[
            "aI+OeCz8xrQ",
            "bt3CTBKmGyI",
            "rcrVFJsQWRY",
            "nsYoNRywwNg",
            "3qxgM4ezETA",
            "62KCwEMmzcM",
            "6UgtwV+0zb4",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
}
