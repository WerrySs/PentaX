// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn create(p: &mut Program) {
    p.ptr(7, 0);
    p.path(6, NAME);
    p.call(0);
    p.expect(0);
}

#[test]
fn native_event_queue_exports_refuse_other_versions_and_namespaces() {
    let mut p = Program::new();
    p.reg(7, 0);
    p.reg(6, 0);
    p.call(0);
    let (code, _) = p.finish(None);
    for (module, library, version, expected) in [
        ("libkernel", "libkernel", 2, "import-version-mismatch"),
        ("libkernel", "Posix", 1, "unavailable-function"),
        ("unrelated-module", "libkernel", 1, "unavailable-function"),
    ] {
        let source = scoped_system_image(&code, &["D0OdFMjp46I"], module, library, version);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], expected);
        assert!(!fields.contains_key("value"));
    }
}

#[test]
fn native_event_queue_wait_resumes_actual_child_on_trigger_and_close() {
    let imports = [
        "D0OdFMjp46I",
        "WDszmSbWuDk",
        "F6e0kwo4cnk",
        "fzyMKs9kim0",
        "jpFjmgAC5AE",
        "6UgtwV+0zb4",
        "kwGyyjohI50",
    ];
    for closed in [false, true] {
        let mut p = Program::new();
        create(&mut p);
        p.load(7, 0);
        p.reg(6, 7);
        p.call(1);
        p.expect(0);
        p.set(24, 0);
        p.set(32, 0);
        p.reg(0, 0xa5a5_a5a5_7fff_ffff);
        p.save(56);
        p.ptr(7, 16);
        p.reg(6, 0);
        p.path(2, 0xc00);
        p.ptr(1, 0);
        p.reg(8, 0);
        p.call(5);
        p.expect(0);
        p.word(24, 1); // Actual child has parked at its event wait.
        p.load(7, 0);
        if closed {
            p.call(4);
        } else {
            p.reg(6, 7);
            p.reg(2, 99);
            p.call(2);
        }
        p.expect(0);
        p.load(0, 24);
        p.code.extend([0x48, 0x83, 0xf8, 2, 0x75, 0xf5]);
        p.word(32, if closed { 0x8002_0009 } else { 0 });
        p.word(
            56,
            0xa5a5_a5a5_0000_0000 | if closed { 0x8002_0009 } else { 1 },
        );
        if !closed {
            for (offset, value) in [(64, 7), (72, 0x21fff5), (80, 99), (88, 99)] {
                p.word(offset, value);
            }
            p.ptr(7, 64);
            p.call(6);
            p.expect(99);
            p.load(7, 0);
            p.call(4);
            p.expect(0);
        }
        let (mut code, calls) = p.finish(None);
        assert!(code.len() < 0x400);
        code.resize(0x400, 0xcc);
        let mut child = vec![0x53, 0x48, 0x89, 0xfb]; // RBX retains original parent buffer.
        child.extend([0x48, 0xc7, 0x43, 24, 1, 0, 0, 0]);
        child.extend([
            0x48, 0x8b, 0x3b, 0x48, 0x8d, 0x73, 64, 0xba, 1, 0, 0, 0, 0x48, 0x8d, 0x4b, 56, 0x45,
            0x31, 0xc0,
        ]);
        rip(&mut child, 0xc00, &[0xff, 0x15], 0xf18);
        child.extend([
            0x48, 0x89, 0x43, 32, 0x48, 0xc7, 0x43, 24, 2, 0, 0, 0, 0x5b, 0xc3,
        ]);
        code.extend(child);
        let source = image(&code, &imports);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], (calls + 1).to_string());
    }
}

#[test]
fn native_event_queue_wait_honors_timeout_and_real_timer_deadline() {
    let imports = ["D0OdFMjp46I", "R74tt43xP6k", "fzyMKs9kim0", "jpFjmgAC5AE"];
    let mut p = Program::new();
    create(&mut p);
    p.reg(0, 0xa5a5_a5a5_0000_1388);
    p.save(24); // Read-only uint32 timeout.
    p.reg(0, 0xa5a5_a5a5_7fff_ffff);
    p.save(32);
    p.load(7, 0);
    p.ptr(6, 64);
    p.reg(2, 1);
    p.ptr(1, 32);
    p.ptr(8, 24);
    p.call(2);
    p.expect(0x8002_003c);
    p.word(24, 0xa5a5_a5a5_0000_1388);
    p.word(32, 0xa5a5_a5a5_0000_0000);
    p.set(48, 0);
    p.reg(0, 5_000_000);
    p.save(56);
    p.load(7, 0);
    p.reg(6, 7);
    p.ptr(2, 48);
    p.reg(1, 0x1234);
    p.call(1);
    p.expect(0);
    p.load(7, 0);
    p.ptr(6, 64);
    p.reg(2, 1);
    p.ptr(1, 32);
    p.reg(8, 0);
    p.call(2);
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0001);
    for (offset, value) in [(64, 7), (72, 0x11fff1), (80, 0), (88, 0x1234)] {
        p.word(offset, value);
    }
    p.set(24, 0);
    p.load(7, 0);
    p.ptr(6, 64);
    p.reg(2, 1);
    p.ptr(1, 32);
    p.ptr(8, 24);
    p.call(2);
    p.expect(0x8002_003c);
    p.load(7, 0);
    p.call(3);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let source = image(&code, &imports);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_event_queue_getters_read_exact_source_field_widths() {
    let imports = [
        "vz+pg2zdopI",
        "mJ7aghmgvfc",
        "23CPPI1tyBY",
        "kwGyyjohI50",
        "Q0qr9AyqJSk",
        "Uu-iDFC9aUc",
    ];
    let expected = [
        0x8765_4321_0fed_cba9,
        u64::MAX,
        0xffff_fff5,
        0xf123_4567_89ab_cdef,
        0xdead_beef,
        0x89ab_cdef,
    ];
    let mut p = Program::new();
    for (offset, value) in [
        (64, u64::MAX),
        (72, 0xdead_beef_4000_fff5),
        (80, 0xf123_4567_89ab_cdef),
        (88, 0x8765_4321_0fed_cba9),
    ] {
        p.reg(0, value);
        p.save(offset);
    }
    for (slot, expected) in expected.into_iter().enumerate() {
        p.ptr(7, 64);
        p.call(slot as i32);
        p.expect(expected);
        p.reg(7, 0);
        p.call(slot as i32);
        p.expect(0);
    }
    let (code, calls) = p.finish(None);
    let source = image(&code, &imports);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
