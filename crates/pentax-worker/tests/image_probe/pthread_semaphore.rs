// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_posix_semaphore_family_preserves_prefix_and_thread_errno() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.set_errno(71);
    p.ptr(7, 16);
    p.reg(6, 0x1234_5678);
    p.reg(2, 1);
    p.call(1);
    p.expect(0);
    p.word(16, 0x09fa);
    p.word(24, 0x1234_5678_0000_0001);
    p.errno(71);
    p.ptr(7, 16);
    p.call(3);
    p.expect(0);
    p.ptr(7, 16);
    p.call(4);
    p.expect(0xffff_ffff);
    p.errno(35);
    p.word(24, 0x1234_5678_0000_0000);
    p.ptr(7, 16);
    p.call(7);
    p.expect(0);
    p.set(64, 0);
    p.set(72, 0);
    p.ptr(7, 16);
    p.ptr(6, 64);
    p.call(6); // Past absolute deadline still acquires an available count.
    p.expect(0);
    p.errno(35);
    p.ptr(7, 16);
    p.reg(6, 0);
    p.call(5);
    p.expect(0xffff_ffff);
    p.errno(60);
    p.ptr(7, 16);
    p.ptr(6, 64);
    p.call(6);
    p.expect(0xffff_ffff);
    p.errno(60);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(32);
    p.ptr(7, 16);
    p.ptr(6, 32);
    p.call(8);
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0000);
    p.ptr(7, 16);
    p.call(2);
    p.expect(0);
    p.word(16, 0);
    p.word(24, 0);
    p.ptr(7, 16);
    p.call(7);
    p.expect(0xffff_ffff);
    p.errno(22);
    let (code, calls) = p.finish(None);
    let imports = [
        ("9BcDykPmo1I", false),
        ("pDuPEf3m4fI", true),
        ("cDW233RAwWo", true),
        ("YCV5dGGBcCo", true),
        ("WBWzsRifCEA", true),
        ("4SbrhCozqQU", true),
        ("w5IHyvahg-o", true),
        ("IKP8typ0QUk", true),
        ("Bq+LRV-N6Hk", true),
    ];
    for (library, version) in [("Posix", 1), ("libScePosix", 1), ("libScePosix", 2)] {
        let source = posix::scoped_image_with_library(&code, &imports, version, library);
        let (status, fields, text) = probe(&source);
        if version == 1 {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        }
    }
}

#[test]
fn native_direct_pthread_semaphores_use_kernel_errors_and_same_count_layout() {
    let mut p = Program::new();
    p.call(7);
    p.save(0);
    p.set_errno(71);
    p.ptr(7, 16);
    p.reg(6, 1);
    p.reg(2, 1);
    p.reg(1, 0);
    p.call(0);
    p.expect(0x8002_0016);
    p.errno(71);
    p.ptr(7, 16);
    p.reg(6, 0);
    p.reg(2, 1);
    p.reg(1, 0);
    p.call(0);
    p.expect(0);
    p.word(16, 0x09fa);
    p.word(24, 1);
    p.ptr(7, 16);
    p.call(2);
    p.expect(0);
    p.ptr(7, 16);
    p.call(3);
    p.expect(0x8002_0023);
    p.ptr(7, 16);
    p.reg(6, 0);
    p.call(4);
    p.expect(0x8002_003c);
    p.ptr(7, 16);
    p.call(5);
    p.expect(0);
    p.set(32, 0);
    p.ptr(7, 16);
    p.ptr(6, 32);
    p.call(6);
    p.expect(0);
    p.word(32, 1);
    p.ptr(7, 16);
    p.call(1);
    p.expect(0);
    p.errno(71);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "GEnUkDZoUwY",
            "Vwc+L05e6oE",
            "C36iRE0F5sE",
            "H2a+IN9TP0E",
            "fjN6NQHhK8k",
            "aishVAiFaYM",
            "DjpBvGlaWbQ",
            "9BcDykPmo1I",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
