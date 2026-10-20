// SPDX-License-Identifier: GPL-2.0-only
//! Original guest consumers of all exact key exports and full pointer values.
use super::*;

fn check(main: &Input, calls: u64) {
    let (status, fields, text) = probe(main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn native_prefixed_posix_keys_use_the_same_state_and_keep_versions_exact() {
    let mut p = Program::new();
    p.ptr(7, 8);
    p.reg(6, 0);
    p.call(0);
    p.expect(0);
    p.load(7, 8);
    p.reg(6, 73);
    p.call(1);
    p.expect(0);
    p.load(7, 8);
    p.call(2);
    p.expect(73);
    p.load(7, 8);
    p.call(3);
    p.expect(0);
    p.load(7, 8);
    p.call(2);
    p.expect(0);
    p.load(7, 8);
    p.call(3);
    p.expect(22);
    let (code, calls) = p.finish(None);
    let imports = [
        ("mqULNdimTn0", true),
        ("WrOLvHU0yQM", true),
        ("eoht7mQOCmo", false),
        ("6BpEZuDT7YI", true),
    ];
    let main = posix::scoped_image_with_library(&code, &imports, 1, "libScePosix");
    check(&main, calls);
    let changed = posix::scoped_image_with_library(&code, &imports, 2, "libScePosix");
    let (status, fields, text) = probe(&changed);
    assert_eq!(status, 2, "{text}");
    assert_ne!(fields["code"], "outer-return");
    assert_eq!(fields["primary_native_instructions"], "0");
    assert!(!fields.contains_key("value"));
}
fn pointer_value(p: &mut Program, offset: u8, value: u64) {
    p.ptr(10, offset);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x48, 0x8b, 0]);
    p.expect(value);
}

#[test]
fn native_pthread_keys_preserve_four_byte_outputs_pointer_values_and_deleted_generations() {
    // All four direct exports and all four independently scoped POSIX exports.
    let mut p = Program::new();
    p.reg(0, 0x1122_3344_5566_7788);
    p.save(8);
    p.set(32, 42);
    p.set(40, 73);
    p.ptr(7, 8);
    p.reg(6, 0);
    p.call(0);
    p.expect(0);
    p.word(8, 0x1122_3344_0000_0000);
    p.load(7, 8);
    p.ptr(6, 32);
    p.call(2);
    p.expect(0);
    p.load(7, 8);
    p.call(7);
    pointer_value(&mut p, 32, 42);
    p.load(7, 8);
    p.call(5);
    p.expect(0);
    p.load(7, 8);
    p.call(3);
    p.expect(0);
    p.ptr(7, 8);
    p.reg(6, 0);
    p.call(4);
    p.expect(0);
    p.word(8, 0x1122_3344_0000_0000);
    p.load(7, 8);
    p.call(3);
    p.expect(0);
    p.load(7, 8);
    p.ptr(6, 40);
    p.call(6);
    p.expect(0);
    p.load(7, 8);
    p.call(7);
    pointer_value(&mut p, 40, 73);
    p.load(7, 8);
    p.call(1);
    p.expect(0);
    p.load(7, 8);
    p.call(5);
    p.expect(22);
    p.load(7, 8);
    p.call(1);
    p.expect(0x8002_0016);
    let (code, calls) = p.finish(Some(32));
    let main = posix::scoped_image(
        &code,
        &[
            ("geDaqgH9lTg", false),
            ("PrdHuuDekhY", false),
            ("+BzXYkqYeLE", false),
            ("eoht7mQOCmo", false),
            ("mqULNdimTn0", true),
            ("6BpEZuDT7YI", true),
            ("WrOLvHU0yQM", true),
            ("0-KXaS70xy4", true),
        ],
        1,
    );
    check(&main, calls);
}

#[test]
fn native_pthread_key_errors_keep_errno_unchanged_and_getspecific_has_a_pointer_domain() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.errno(0);
    p.set_errno(37);
    p.reg(7, 0);
    p.reg(6, 0);
    p.call(1);
    p.expect(0x8002_0016);
    p.errno(37);
    p.reg(7, 0);
    p.call(2);
    p.expect(22);
    p.errno(37);
    p.set(8, 85);
    p.ptr(7, 8);
    p.reg(6, 0);
    p.call(2);
    p.expect(0);
    p.word(8, 0);
    p.errno(37);
    p.reg(7, 256);
    p.reg(6, 42);
    p.call(3);
    p.expect(22);
    p.errno(37);
    p.reg(7, u64::MAX);
    p.call(7);
    p.expect(0x8002_0016);
    p.errno(37);
    p.reg(7, u64::MAX);
    p.call(5);
    p.expect(0);
    p.errno(37);
    p.reg(7, 256);
    p.call(4);
    p.expect(22);
    p.errno(37);
    p.load(7, 8);
    p.call(6);
    p.expect(0);
    p.errno(37);
    p.load(7, 8);
    p.call(5);
    p.expect(0);
    p.errno(37);
    let (code, calls) = p.finish(None);
    let main = posix::scoped_image(
        &code,
        &[
            ("9BcDykPmo1I", false),
            ("geDaqgH9lTg", false),
            ("mqULNdimTn0", true),
            ("WrOLvHU0yQM", true),
            ("6BpEZuDT7YI", true),
            ("0-KXaS70xy4", true),
            ("PrdHuuDekhY", false),
            ("+BzXYkqYeLE", false),
        ],
        1,
    );
    check(&main, calls);
}

#[test]
fn native_pthread_key_bad_output_backing_is_never_converted_to_an_errno_result() {
    for slot in [0, 1] {
        let mut p = Program::new();
        p.reg(7, u64::MAX);
        p.reg(6, 0);
        p.call(slot);
        let (code, calls) = p.finish(None);
        let main = posix::scoped_image(&code, &[("geDaqgH9lTg", false), ("mqULNdimTn0", true)], 1);
        let (status, fields, text) = probe(&main);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service");
        assert_eq!(fields["service_stops"], calls.to_string());
        assert!(!fields.contains_key("value"));
    }
}
