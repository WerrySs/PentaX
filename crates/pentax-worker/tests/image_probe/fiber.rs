// SPDX-License-Identifier: GPL-2.0-only

//! Original CPU consumer of actual fiber ABI storage and callee-stack arguments.
use super::*;

fn pointer(p: &mut Program, reg: u8, offset: u32) {
    p.code.extend([
        0x48 | if reg >= 8 { 4 } else { 0 },
        0x8d,
        0x84 | ((reg & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn read(p: &mut Program, offset: u32, expected: u64) {
    p.code.extend([0x48, 0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.expect(expected);
}
fn initialize(p: &mut Program, slot: i32) {
    pointer(p, 7, 128);
    p.path(6, NAME);
    p.path(2, 0x800);
    p.reg(1, 0x1122_3344_5566_7788);
    pointer(p, 8, 1024);
    p.reg(9, 512);
    p.call(slot);
    p.expect(0);
}

#[test]
fn native_fiber_lifecycle_consumes_eight_and_nine_arguments_in_exact_scopes() {
    let mut p = Program::new();
    // This frame holds the public object,128-byte option/info buffers and the
    // caller's512-byte context. Three outgoing stack arguments occupy0..24.
    p.code.extend([0x48, 0x81, 0xec, 0, 0x10, 0, 0]);
    pointer(&mut p, 7, 512);
    p.call(2);
    p.expect(0);
    read(&mut p, 512, 0xbb40_e64d);
    pointer(&mut p, 0, 512);
    p.save(0); // Callee RSP+8: option pointer.
    p.reg(0, 0xffff_ffff_0350_0000);
    p.save(8); // Callee RSP+16: uint32 build version.
    initialize(&mut p, 0);
    read(&mut p, 128, 0x0000_0002_def1_649c);
    read(&mut p, 144, 0x1122_3344_5566_7788);
    read(&mut p, 160, 512);
    read(&mut p, 200, 0); // No saved native/host context pointer.
    read(&mut p, 208, 0x100);
    read(&mut p, 328, 0xb375_92a0_0000_0000);
    read(&mut p, 1024, 0x7149_f2ca_7149_f2ca);
    pointer(&mut p, 7, 128);
    pointer(&mut p, 6, 768);
    p.call(4);
    p.expect(0);
    read(&mut p, 768, 128);
    read(&mut p, 784, 0x1122_3344_5566_7788);
    read(&mut p, 800, 512);
    pointer(&mut p, 7, 128);
    p.path(6, MISSING);
    p.call(5);
    p.expect(0);
    read(&mut p, 168, u64::from_le_bytes(*b"/app0/as"));
    pointer(&mut p, 7, 128);
    p.call(3);
    p.expect(0);
    pointer(&mut p, 7, 128);
    p.call(3);
    p.expect(0x8059_0006);
    p.reg(0, 0xffff_ffff_0000_0042);
    p.save(8); // Internal's eighth argument is flags, not version.
    p.reg(0, 0xffff_ffff_034f_ffff);
    p.save(16); // Internal's ninth argument is build version.
    initialize(&mut p, 1);
    read(&mut p, 208, 0x42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 0x10, 0, 0]);
    let (code, calls) = p.finish(None);
    let nids = [
        "hVYD7Ou2pCQ",
        "7+OJIpko9RY",
        "asjUJJ+aa8s",
        "JeNX5F-NzQU",
        "uq2Y5BFz0PE",
        "JzyT91ucGDc",
    ];
    for (module, library, version, expected) in [
        ("Fiber", "Fiber", 1, "exit"),
        ("libSceFiber", "Fiber", 1, "exit"),
        ("Fiber", "libSceFiber", 1, "exit"),
        ("libSceFiber", "libSceFiber", 1, "exit"),
        ("libkernel", "libkernel", 1, "unavailable-function"),
        ("libSceFiber", "libSceFiber", 2, "import-version-mismatch"),
    ] {
        let source = scoped_system_image(&code, &nids, module, library, version);
        let (status, fields, text) = probe(&source);
        if expected == "exit" {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], expected, "{text}");
        }
    }
}
