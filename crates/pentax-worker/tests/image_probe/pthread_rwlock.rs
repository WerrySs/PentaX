// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_rwlock_lifecycle_retains_reader_and_writer_state_and_guest_prefix() {
    let mut p = Program::new();
    p.set(0, 0);
    p.ptr(7, 0);
    p.reg(6, u64::MAX); // Pinned source ignores the attribute argument.
    p.reg(2, 0);
    p.call(0);
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x83, 0x8b, 0xfc, 0, 0, 0, 0x42]);
    for slot in [1, 2] {
        // Writer can also take a recursive reader.
        p.ptr(7, 0);
        p.call(slot);
        p.expect(0);
    }
    p.ptr(7, 0);
    p.call(3);
    p.expect(0); // Release reader first.
    p.ptr(7, 0);
    p.call(4);
    p.expect(I32_BUSY);
    p.ptr(7, 0);
    p.call(1);
    p.expect(0x8002_000b); // Writer still held.
    p.ptr(7, 0);
    p.call(3);
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x8b, 0x83, 0xfc, 0, 0, 0]);
    p.expect(0x42);
    p.ptr(7, 0);
    p.call(4);
    p.expect(0);
    p.word(0, 0);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "6ULAa0fq4jA",
            "mqdNorrB+gI",
            "Ox9i0c7L5w0",
            "+L98PIbGttk",
            "BB+kb08Tl9A",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
}
