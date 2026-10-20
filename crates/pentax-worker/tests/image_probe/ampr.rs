// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_apr_submit_and_wait_use_posix_results_and_retained_completion_receipts() {
    let mut p = Program::new();
    p.reg(0, 0);
    for offset in [0, 8, 16, 80] {
        p.save(offset);
    }
    p.ptr(7, 0);
    p.reg(6, u64::MAX); // The source ignores the argument word.
    p.ptr(2, 64);
    p.ptr(1, 80);
    p.call(0);
    p.expect(0);
    p.word(64, 0);
    p.word(80, 1);
    p.reg(0, u64::MAX);
    p.save(64);
    p.reg(7, (7 << 32) | 1);
    p.call(1);
    p.expect(0);
    p.word(64, 0);
    p.reg(7, 1);
    p.call(1);
    p.expect(u64::from(u32::MAX)); // POSIX int32 -1, not a direct kernel error.
    p.ptr(7, 0);
    p.call(2);
    p.expect(0); // Plain submit allocates no receipt.
    p.ptr(7, 0);
    p.ptr(2, 80);
    p.call(3);
    p.expect(0);
    p.word(80, 2);
    p.reg(7, 2);
    p.call(1);
    p.expect(0);
    p.reg(7, 0);
    p.call(2);
    p.expect(u64::from(u32::MAX));
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &["ASoW5WE-UPo", "rqwFKI4PAiM", "eE4Szl8sil8", "qvMUCyyaCSI"],
        "libkernel",
        "libkernel",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

fn expect_stack_pointer(p: &mut Program, offset: u8) {
    p.ptr(10, offset);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
}

#[test]
fn native_ampr_read_records_use_seventh_stack_argument_and_checked_lifecycle_state() {
    let mut p = Program::new();
    p.ptr(7, 16);
    p.call(0);
    p.expect(0);
    p.ptr(7, 16);
    p.ptr(6, 64);
    p.reg(2, 48);
    p.call(1);
    p.expect(0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    for offset in [64, 72, 80, 88, 96, 104] {
        p.save(offset);
    }
    p.set(112, 85);
    p.reg(0, 0x1_0000_0000);
    p.save(0); // Argument seven at caller RSP = captured callee RSP+8.
    p.ptr(7, 16);
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX);
    p.reg(1, (1 << 40) | 99);
    p.ptr(8, 112);
    p.reg(9, 8);
    p.call(2);
    p.expect(0);
    p.word(16, (24_u64 << 32) | 0x1_0000);
    p.word(24, (48_u64 << 32) | 1);
    p.word(64, 0x17);
    p.word(72, 0);
    p.word(80, 0);
    p.word(112, 85); // Append accepts an unknown ID without reading into destination.
    p.set(0, 0);
    p.ptr(7, 16);
    p.reg(1, 7);
    p.reg(8, 0); // Null destination is numerically valid when only queuing.
    p.reg(9, 1);
    p.call(2);
    p.expect(0);
    p.word(16, (44_u64 << 32) | 0x1_0000);
    p.word(24, (48_u64 << 32) | 2);
    p.word(88, 0x17);
    p.word(104, 0xa5a5_a5a5_0000_0000);
    p.ptr(7, 16);
    p.call(3);
    p.expect(0);
    p.word(16, 0x1_0000);
    p.word(24, 48_u64 << 32);
    p.word(64, 0x17);
    p.reg(6, 0);
    p.reg(2, 1);
    p.reg(1, 0x1_0000_0000);
    p.call(4);
    p.expect(24);
    p.reg(2, 0);
    p.call(4);
    p.expect(0x8002_0016); // The source u64 helper zero-extends kernel error bits.
    p.ptr(7, 16);
    p.call(5);
    expect_stack_pointer(&mut p, 64);
    p.word(24, 0);
    p.word(32, 0);
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &[
            "8aI7R7WaOlc",
            "N-FSPA4S3nI",
            "mQ16-QdKv7k",
            "baQO9ez2gL4",
            "vWU-odnS+fU",
            "ULvXMDz56po",
        ],
        "libSceAmpr",
        "libSceAmpr",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_ampr_lifecycle_retains_actual_bindings_and_exact_scope_error_domains() {
    let mut p = Program::new();
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(24);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    for offset in [0, 8, 16] {
        p.word(offset, 0);
    }
    p.word(24, 0xa5a5_a5a5_a5a5_a5a5);
    p.ptr(7, 0);
    p.ptr(6, 64);
    p.reg(2, (1 << 40) | 64); // Source size argument is uint32_t.
    p.call(1);
    p.expect(0);
    p.call(1);
    p.expect(0x8002_0010);
    p.reg(0, (12_u64 << 32) | 0x1234_5678);
    p.save(0);
    p.reg(0, (96_u64 << 32) | 7);
    p.save(8);
    p.ptr(0, 96);
    p.save(16);
    p.call(2); // Retained binding wins over directly modified visible fields.
    p.expect(0);
    p.word(0, 0x1234_5678);
    p.word(8, 64 << 32);
    p.call(4);
    p.expect(64);
    p.call(5);
    expect_stack_pointer(&mut p, 64);
    p.call(7);
    p.expect(0);
    p.call(3);
    expect_stack_pointer(&mut p, 64);
    p.word(8, 0);
    p.word(16, 0);
    p.word(0, 0x1234_5678);
    p.call(6);
    p.expect(0);
    p.reg(7, 0);
    p.call(1);
    p.expect(0x8002_0016);
    p.call(2);
    p.expect(0x8002_0001);
    p.call(0);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let nids = [
        "8aI7R7WaOlc",
        "N-FSPA4S3nI",
        "baQO9ez2gL4",
        "ULvXMDz56po",
        "tZDDEo2tE5k",
        "RPCAhx-aabE",
        "GuchCTefuZw",
        "GnxKOHEawhk",
    ];
    for (module, library, version, expected) in [
        ("Ampr", "Ampr", 1, "exit"),
        ("libSceAmpr", "Ampr", 1, "exit"),
        ("Ampr", "libSceAmpr", 1, "exit"),
        ("libSceAmpr", "libSceAmpr", 1, "exit"),
        ("libSceAmpr", "libSceAmpr", 2, "import-version-mismatch"),
        ("libkernel", "libkernel", 1, "unavailable-function"),
    ] {
        let input = scoped_system_image(&code, &nids, module, library, version);
        let (status, fields, text) = probe(&input);
        if expected == "exit" {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], expected);
        }
    }
}

#[test]
fn native_ampr_reserved_state_and_live_queries_preserve_exact_unrelated_words() {
    let mut p = Program::new();
    let first = 0x1122_3344_5566_7788;
    let second = 0xaabb_ccdd_0011_2233;
    let sentinel = 0xa5a5_a5a5_a5a5_a5a5;
    p.reg(0, first);
    p.save(0);
    p.reg(0, second);
    p.save(8);
    p.reg(0, sentinel);
    for offset in [16, 24, 32, 40] {
        p.save(offset);
    }
    p.ptr(7, 0);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(1);
    p.expect(0);
    p.word(0, first);
    p.word(8, second);
    p.word(16, sentinel);
    p.word(24, 0);
    p.word(32, 0);
    p.word(40, sentinel);
    for (slot, value) in [(3, 0x5566_7788), (4, 0x11_2233), (5, 0x1122_3344)] {
        p.call(slot);
        p.expect(value);
    }
    p.call(2);
    p.expect(0);
    p.word(0, first);
    p.reg(0, sentinel);
    for offset in [56, 64, 72, 80] {
        p.save(offset);
    }
    p.ptr(6, 56);
    p.ptr(2, 72);
    p.call(1);
    p.expect(0);
    p.word(56, 0);
    p.word(64, sentinel);
    p.word(72, 0);
    p.word(80, sentinel);
    p.reg(0, sentinel);
    p.save(56);
    p.ptr(2, 56);
    p.call(1);
    p.expect(0);
    p.word(56, 0);
    p.word(64, sentinel);
    p.call(0);
    p.expect(0);
    p.word(0, 0);
    p.word(8, 0);
    p.word(16, 0);
    p.word(40, sentinel);
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &[
            "8aI7R7WaOlc",
            "a8uLzYY--tM",
            "Qs1xtplKo0U",
            "VEDMaQmJZng",
            "gzndltBEzWc",
            "GnxKOHEawhk",
        ],
        "libSceAmpr",
        "libSceAmpr",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
