// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_base_hardware_queries_publish_exact_words_and_source_alias_order() {
    let mut p = Program::new();
    p.reg(0, u64::MAX);
    p.call(0);
    p.expect(0);
    for (second, expected) in [
        (8, 0xa5a5_a5a5_0000_0002),
        (2, 0xa5a5_0000_0000_0002),
        (0, 0xa5a5_a5a5_0000_0000),
    ] {
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(0);
        p.save(8);
        p.ptr(7, 0);
        p.ptr(6, second);
        p.call(1);
        p.expect(0);
        p.word(0, expected);
        if second == 8 {
            p.word(8, 0xa5a5_a5a5_0000_0000);
        }
    }
    let (code, calls) = p.finish(None);
    let original = image(&code, &["tU5e3f9gSiU", "NH6xARDOVv8"]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
}
