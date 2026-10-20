// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn open(p: &mut Program, bus: u64, expected: u64) {
    p.reg(7, 255);
    p.reg(6, bus);
    p.reg(2, 0);
    p.reg(1, u64::MAX);
    p.call(0);
    p.expect(expected);
}

#[test]
fn native_video_registration_changes_and_releases_the_same_owned_port_buffers() {
    for (module, library, version) in [
        ("VideoOut", "VideoOut", 1),
        ("libSceVideoOut", "VideoOut", 1),
        ("VideoOut", "libSceVideoOut", 1),
        ("libSceVideoOut", "libSceVideoOut", 1),
        ("VideoOut", "VideoOut", 2),
    ] {
        let mut p = Program::new();
        open(&mut p, 0, 1);
        for (offset, value) in [
            (0, 0xdead_beef_0000_0000),
            (8, 0),
            (16, 0x1000_0000),
            (24, 0),
            (32, u64::MAX),
            (40, u64::MAX),
            (48, 0),
            (56, 129_u64 << 32),
            (64, 65),
            (72, 40),
            (80, 0x8000_0000_2200_0000),
            (88, 0),
            (96, 0),
            (104, 0),
            (112, 0),
            (120, 0),
        ] {
            p.reg(0, value);
            p.save(offset);
        }
        for expected in [0, 0x8029_000a] {
            p.reg(7, 1);
            p.reg(6, 0);
            p.reg(2, 3);
            p.ptr(1, 16);
            p.reg(8, 1);
            p.ptr(9, 48);
            p.call(1);
            p.expect(expected);
        }
        p.reg(0, 1920_u64 << 32);
        p.save(56);
        p.reg(0, 1080);
        p.save(64);
        p.reg(7, 1);
        p.reg(6, 0);
        p.ptr(2, 48);
        p.reg(1, 0);
        p.call(2);
        p.expect(0);
        for expected in [0, 0x8029_000a] {
            p.reg(7, 1);
            p.reg(6, 0);
            p.call(3);
            p.expect(expected);
        }
        for close_first in [false, true] {
            if close_first {
                p.reg(7, 1);
                p.call(4);
                p.expect(0);
                open(&mut p, 0, 1);
            }
            p.reg(7, 1);
            p.reg(6, 0);
            p.reg(2, 3);
            p.ptr(1, 16);
            p.reg(8, 1);
            p.ptr(9, 48);
            p.call(1);
            p.expect(0);
        }
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &[
                "Up36PTk687E",
                "rKBUtgRrtbk",
                "HuViW4HnrOw",
                "N5KDtkIjjJ4",
                "uquVH4-Du78",
            ],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 1 {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        }
    }
}

#[test]
fn native_video_buffer_attributes_use_real_stack_arguments_and_preserve_option_neighbors() {
    for (module, library, version) in [
        ("VideoOut", "VideoOut", 1),
        ("libSceVideoOut", "VideoOut", 1),
        ("VideoOut", "libSceVideoOut", 1),
        ("libSceVideoOut", "libSceVideoOut", 1),
        ("VideoOut", "VideoOut", 2),
    ] {
        let mut p = Program::new();
        p.reg(0, 0xdead_beef_1234_5678);
        p.save(0);
        p.reg(0, 0xfedc_ba98_7654_3210);
        p.save(8);
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(24);
        p.save(112);
        p.ptr(7, 32);
        p.reg(6, 0x1122_3344_5566_7788);
        p.reg(2, 2);
        p.reg(1, 1920);
        p.reg(8, 1080);
        p.reg(9, 0x8000_0000_0000_0001);
        p.call(0); // Source void result is deliberately ignored.
        for (slot, expected) in [
            (32, 0x0000_0002_0000_0000),
            (40, 0x0000_0780_0000_0000),
            (48, 1080),
            (56, 0x8000_0000_0000_0001),
            (64, 0x1122_3344_5566_7788),
            (72, 0xfedc_ba98_7654_3210),
            (80, 0x1234_5678),
            (88, 0),
            (96, 0),
            (104, 0),
        ] {
            p.word(slot, expected);
        }
        p.ptr(7, 32);
        p.reg(6, 0x4000_0000_0000_0002);
        p.call(1);
        p.word(56, 0xc000_0000_0000_0003);
        p.word(64, 0x1122_3344_5566_7788);
        p.word(24, 0xa5a5_a5a5_a5a5_a5a5);
        p.word(112, 0xa5a5_a5a5_a5a5_a5a5);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["PjS5uASwcV8", "MBlv8YWoyOk"],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 1 {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
            assert!(!fields.contains_key("value"));
        }
    }
}

#[test]
fn native_video_output_status_has_exact_scopes_record_width_and_error_order() {
    for (module, library, version) in [
        ("VideoOut", "VideoOut", 1),
        ("libSceVideoOut", "VideoOut", 1),
        ("VideoOut", "libSceVideoOut", 1),
        ("libSceVideoOut", "libSceVideoOut", 1),
        ("VideoOut", "VideoOut", 2),
    ] {
        let mut p = Program::new();
        open(&mut p, 0, 1);
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(8);
        p.save(64);
        p.reg(7, 1);
        p.ptr(6, 16);
        p.call(1);
        p.expect(0);
        p.word(8, 0xa5a5_a5a5_a5a5_a5a5);
        p.word(16, 0x0000_0001_0000_0002);
        p.word(24, 3);
        for offset in [32, 40, 48, 56] {
            p.word(offset, 0);
        }
        p.word(64, 0xa5a5_a5a5_a5a5_a5a5);
        p.reg(7, 1);
        p.call(2);
        p.expect(0);
        p.reg(7, 1);
        p.ptr(6, 16);
        p.call(1);
        p.expect(0x8029_000b);
        p.reg(7, 9);
        p.reg(6, 0);
        p.call(1);
        p.expect(0x8029_0002);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["Up36PTk687E", "utPrVdxio-8", "uquVH4-Du78"],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 1 {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
            assert!(!fields.contains_key("value"));
        }
    }
}

#[test]
fn native_video_ports_retain_real_lifecycle_and_exact_scoped_configuration() {
    for (module, library, version) in [
        ("VideoOut", "VideoOut", 1),
        ("libSceVideoOut", "VideoOut", 1),
        ("VideoOut", "libSceVideoOut", 1),
        ("libSceVideoOut", "libSceVideoOut", 1),
        ("VideoOut", "VideoOut", 2),
    ] {
        let mut p = Program::new();
        open(&mut p, 0, 1);
        open(&mut p, 0, 0x8029_0009);
        open(&mut p, 1, 2);
        p.reg(7, 2);
        p.reg(6, 1);
        p.call(2);
        p.expect(0);
        p.reg(7, 2);
        p.reg(6, 2);
        p.call(3);
        p.expect(0);
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(24);
        p.save(96);
        p.ptr(7, 32);
        p.call(4);
        p.expect(0);
        p.word(24, 0xa5a5_a5a5_a5a5_a5a5);
        p.word(32, 0);
        p.word(88, 0);
        p.word(96, 0xa5a5_a5a5_a5a5_a5a5);
        for (slot, mode, expected) in [(5, 1, 1), (5, 15, 0), (6, 15, 0x8029_0019), (6, 1, 0)] {
            p.reg(7, 1);
            p.reg(6, mode);
            p.ptr(2, 32);
            p.reg(1, 0);
            p.reg(8, 0);
            p.call(slot);
            p.expect(expected);
        }
        p.reg(7, 2);
        p.call(1);
        p.expect(0); // Releases the master's retained slave reference.
        p.reg(7, 1);
        p.call(1);
        p.expect(0);
        open(&mut p, 0, 1); // New generation, same source numeric handle.
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &[
                "Up36PTk687E",
                "uquVH4-Du78",
                "WkYtyOg30do",
                "CBiu4mCE1DA",
                "+I4K03i3EL0",
                "Nv8c-Kb+DUM",
                "w0hLuNarQxY",
            ],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 1 {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
            assert!(!fields.contains_key("value"));
        }
    }
}
