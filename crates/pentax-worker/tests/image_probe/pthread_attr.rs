// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_thread_attributes_roundtrip_full_stack_values_and_signed_priority() {
    let mut p = Program::new();
    p.set(0, 0);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x83, 0x4b, 0x38, 0x42]); // Write the owned reserved prefix.
    p.ptr(7, 0);
    p.ptr(6, 16);
    p.ptr(2, 24);
    p.call(1);
    p.expect(0);
    p.word(16, 0);
    p.word(24, 0x100000);
    p.ptr(7, 0);
    p.reg(6, 0x2345_6789_abcd);
    p.reg(2, 0x1_0000_4000);
    p.call(2);
    p.expect(0);
    p.ptr(7, 0);
    p.ptr(6, 16);
    p.ptr(2, 24);
    p.call(1);
    p.expect(0);
    p.word(16, 0x2345_6789_abcd);
    p.word(24, 0x1_0000_4000);
    p.reg(0, (-731_i64) as u64);
    p.save(32);
    p.ptr(7, 0);
    p.ptr(6, 32);
    p.call(3);
    p.expect(0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(32);
    p.ptr(7, 0);
    p.ptr(6, 32);
    p.call(4);
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0000 | u64::from((-731_i32) as u32));
    p.load(11, 0);
    p.code.extend([0x41, 0x8b, 0x43, 0x38]);
    p.expect(0x42);
    p.ptr(7, 0);
    p.call(5);
    p.expect(0);
    p.word(0, 0);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "nsYoNRywwNg",
            "-quPa4SEJUw",
            "Bvn74vj6oLo",
            "DzES9hQF4f4",
            "FXPWHNk8Of0",
            "62KCwEMmzcM",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
}
