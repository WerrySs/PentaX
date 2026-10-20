// SPDX-License-Identifier: GPL-2.0-only

use super::*;
const SIGNED_OUT: u64 = 0x8055_0006;

#[test]
fn native_np_account_queries_preserve_offline_codes_exact_spans_and_reviewed_scopes() {
    let mut p = Program::new();
    for (slot, result, first, tail) in [
        (0, SIGNED_OUT, 0, 0xa5a5_a5a5_0000_0000),
        (1, SIGNED_OUT, 0, 0xa5a5_a5a5_0000_0000),
        (2, SIGNED_OUT, 0, 0),
        (3, SIGNED_OUT, 0xa5a5_a5a5_0000_0000, 0xa5a5_a5a5_0000_0000),
        (4, 0, 0xa5a5_a5a5_0000_0001, 0xa5a5_a5a5_0000_0001),
        (5, 0, 0xa5a5_a5a5_0000_0000, 0xa5a5_a5a5_0000_0000),
        (6, 0, 0xa5a5_a5a5_a5a5_a500, 0xa5a5_a5a5_a5a5_a500),
    ] {
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        for offset in [0, 8, 16, 24, 32, 40] {
            p.save(offset);
        }
        p.reg(7, u64::MAX);
        p.ptr(6, 0);
        p.call(slot);
        p.expect(result);
        p.word(0, first);
        p.word(
            if slot == 0 {
                32
            } else if slot == 1 {
                16
            } else {
                0
            },
            tail,
        );
        p.word(40, 0xa5a5_a5a5_a5a5_a5a5);
    }
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(0);
    p.reg(7, 999);
    p.reg(6, 0);
    p.ptr(2, 0);
    p.call(7);
    p.expect(SIGNED_OUT);
    p.word(0, 0xa5a5_a5a5_a5a5_a500);
    let (code, calls) = p.finish(None);
    let nids = [
        "p-o74CnoNzY",
        "XDncXQIJUSk",
        "rbknaUjpqWo",
        "JT+t00a3TxA",
        "eQH7nWPcAgc",
        "e-ZuhGEoeC4",
        "Oad3rvY-NJQ",
        "+4DegjBqV1g",
    ];
    for (module, library, version, success) in [
        ("NpManager", "NpManager", 1, true),
        ("libSceNpManager", "NpManager", 1, true),
        ("NpManager", "libSceNpManager", 1, true),
        ("libSceNpManager", "libSceNpManager", 1, true),
        ("libSceNpManager", "libSceNpManager", 2, false),
    ] {
        let input = scoped_system_image(&code, &nids, module, library, version);
        let (status, fields, text) = probe(&input);
        if success {
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
fn native_np_requests_complete_poll_abort_and_reuse_the_deleted_slot() {
    let mut p = Program::new();
    p.call(0);
    p.expect(1);
    p.save(0);
    p.set(32, 24);
    p.set(40, 0);
    p.set(48, 0);
    p.ptr(7, 32);
    p.call(1);
    p.expect(2);
    p.save(8);
    p.load(7, 8);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0x8055_0015);
    p.load(7, 0);
    p.reg(6, u64::MAX);
    p.ptr(2, 16);
    p.call(4);
    p.expect(SIGNED_OUT);
    p.load(7, 8);
    p.reg(6, 0);
    p.call(5);
    p.expect(0);
    p.set(16, 0);
    p.load(7, 8);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0);
    p.word(16, SIGNED_OUT);
    p.load(7, 8);
    p.reg(6, 0);
    p.reg(2, u64::MAX);
    p.call(7);
    p.expect(SIGNED_OUT);
    p.load(7, 8);
    p.reg(6, 1);
    p.ptr(2, 16);
    p.call(4);
    p.expect(0x8055_0003);
    p.word(16, 0);
    p.load(7, 8);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0);
    p.word(16, 0x8055_0003);
    p.load(7, 8);
    p.call(3);
    p.expect(0);
    p.load(7, 8);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0);
    p.word(16, 0x8055_0012);
    p.load(7, 0);
    p.call(2);
    p.expect(0);
    p.call(0);
    p.expect(1);
    p.load(7, 0);
    p.call(2);
    p.expect(0);
    p.load(7, 8);
    p.call(2);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &[
            "GpLQDNKICac",
            "eiqMCt9UshI",
            "S7QTn72PrDw",
            "OzKvTvg3ZYU",
            "2rsFmlGWleQ",
            "KfGZg2y73oM",
            "uqcPJLWL08M",
            "3Tcz5bNCfZQ",
        ],
        "libSceNpManager",
        "libSceNpManager",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_np_premium_checks_zero_exact_result_only_when_completing_a_ready_request() {
    let mut p = Program::new();
    p.set(0, 24);
    p.set(8, 0);
    p.set(16, 0);
    p.ptr(7, 0);
    p.call(0);
    p.expect(1);
    p.save(24);
    p.set(32, 56);
    p.set(40, 0);
    p.set(48, 0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    for offset in [88, 96, 104, 112, 120] {
        p.save(offset);
    }
    p.load(7, 24);
    p.ptr(6, 32);
    p.ptr(2, 88);
    p.call(1);
    p.expect(0);
    for offset in [88, 96, 104, 112] {
        p.word(offset, 0);
    }
    p.word(120, 0xa5a5_a5a5_a5a5_a500);
    p.load(7, 24);
    p.ptr(6, 16);
    p.call(2);
    p.expect(0);
    p.word(16, SIGNED_OUT);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(88);
    p.load(7, 24);
    p.ptr(6, 32);
    p.ptr(2, 88);
    p.call(1);
    p.expect(0x8055_0003);
    p.word(88, 0xa5a5_a5a5_a5a5_a5a5);
    p.load(7, 24);
    p.ptr(6, 16);
    p.call(2);
    p.expect(0);
    p.word(16, 0x8055_0003);
    p.load(7, 24);
    p.call(3);
    p.expect(0);
    p.load(7, 24);
    p.ptr(6, 32);
    p.ptr(2, 88);
    p.call(1);
    p.expect(0x8055_0012);
    p.word(88, 0xa5a5_a5a5_a5a5_a5a5);
    p.load(7, 24);
    p.call(4);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &[
            "eiqMCt9UshI",
            "O80NrhUOPGY",
            "uqcPJLWL08M",
            "OzKvTvg3ZYU",
            "S7QTn72PrDw",
        ],
        "NpManager",
        "NpManager",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
