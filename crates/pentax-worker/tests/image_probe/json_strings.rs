// SPDX-License-Identifier: GPL-2.0-only

//! Original imported native String ownership, C-string and Value lifecycle checks.
use super::*;

const TEXT: &[u8] = b"A\"\\\xff\nZ";

fn source_text(p: &mut Program) {
    let mut bytes = [0; 8];
    bytes[..TEXT.len()].copy_from_slice(TEXT);
    p.store(96, u64::from_le_bytes(bytes));
}

fn string(p: &mut Program, object: u8, expected: &[u8]) {
    p.invoke(26, object);
    p.result(expected.len() as u64);
    p.invoke(25, object);
    // Retain the returned guest pointer in RCX; read the actual bytes, rather
    // than treating a nonzero handle or constructor return as String support.
    p.code.extend([0x48, 0x89, 0xc1]);
    let whole = expected.len() / 8;
    for index in 0..whole {
        p.code.extend([0x48, 0x8b, 0x41, (index * 8) as u8]);
        p.result(u64::from_le_bytes(
            expected[index * 8..index * 8 + 8].try_into().unwrap(),
        ));
    }
    for (index, byte) in expected
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .enumerate()
        .skip(whole * 8)
    {
        p.code.extend([0x0f, 0xb6, 0x41, index as u8]);
        p.result(u64::from(byte));
    }
}

fn value_string(p: &mut Program, object: u8, expected: &[u8]) {
    p.invoke(30, object);
    // Copy the returned wrapper word into a temporary guest wrapper. The
    // underlying impl belongs to the source; this view is never destroyed.
    p.code.extend([0x48, 0x8b, 0, 0x48, 0x89, 0x44, 0x24, 112]);
    string(p, 112, expected);
}

fn passed(p: Program, module: &[u8], library: &[u8]) {
    let image = p.finish(module, library);
    let (status, fields, text) = probe(&image);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_string_assignment_keeps_impl_identity_and_independent_guest_bytes() {
    let mut p = Program::new();
    source_text(&mut p);
    p.source(96);
    p.invoke(22, 0);
    p.pointer(0);
    string(&mut p, 0, TEXT);
    p.invoke(21, 16);
    p.pointer(16);
    p.code
        .extend([0x48, 0x8b, 0x44, 0x24, 16, 0x48, 0x89, 0x44, 0x24, 24]);
    p.source(0);
    p.invoke(24, 16);
    p.pointer(16);
    p.code
        .extend([0x48, 0x8b, 0x44, 0x24, 16, 0x48, 0x3b, 0x44, 0x24, 24]);
    p.checked();
    p.invoke(23, 0);
    p.word(0, 0);
    string(&mut p, 16, TEXT);
    p.source(16);
    p.invoke(24, 16);
    p.pointer(16);
    string(&mut p, 16, TEXT);
    p.code.extend([0x31, 0xff]);
    p.call(26);
    p.result(0);
    p.code.extend([0x31, 0xff]);
    p.call(25);
    p.code.extend([0x0f, 0xb6, 0]);
    p.result(0);
    p.argument(u64::MAX);
    p.code.extend([0x31, 0xff]);
    p.call(22);
    p.result(0);
    p.invoke(23, 16);
    p.word(16, 0);
    passed(p, b"Json2", b"Json2");
}

#[test]
fn native_observed_value_strings_copy_clear_and_convert_through_one_owner() {
    let mut p = Program::new();
    source_text(&mut p);
    p.source(96);
    p.invoke(27, 0);
    p.pointer(0);
    value_string(&mut p, 0, TEXT);
    p.source(0);
    p.invoke(12, 32);
    p.pointer(32);
    p.invoke(21, 64);
    p.source(64);
    p.invoke(31, 32);
    string(&mut p, 64, TEXT);
    p.argument(99);
    p.invoke(5, 0);
    p.invoke(8, 0);
    p.result(2);
    value_string(&mut p, 32, TEXT);
    p.source(32);
    p.invoke(13, 0);
    p.pointer(0);
    p.invoke(14, 32);
    p.invoke(8, 32);
    p.result(0);
    value_string(&mut p, 0, TEXT);
    p.argument(5);
    p.invoke(7, 32);
    value_string(&mut p, 32, b"");
    p.invoke(14, 0);
    value_string(&mut p, 0, b"");
    p.source(64);
    p.invoke(28, 0);
    p.pointer(0);
    p.invoke(23, 64);
    value_string(&mut p, 0, TEXT);
    p.invoke(30, 0);
    p.code.extend([0x48, 0x89, 0xc6]);
    p.invoke(29, 32);
    p.invoke(14, 0);
    value_string(&mut p, 32, TEXT);
    p.argument(1);
    p.invoke(4, 32);
    p.invoke(8, 32);
    p.result(1);
    p.invoke(14, 0);
    p.invoke(14, 32);
    passed(p, b"libSceJson", b"libSceJson2");
}

#[test]
fn native_string_serialization_preserves_public_raw_quoting_and_integer_forms() {
    let mut p = Program::new();
    source_text(&mut p);
    p.source(96);
    p.invoke(27, 0);
    p.invoke(21, 64);
    p.source(64);
    p.invoke(32, 0);
    p.result(0);
    let mut quoted = vec![b'"'];
    quoted.extend_from_slice(TEXT);
    quoted.push(b'"');
    string(&mut p, 64, &quoted);
    for (setter, argument, expected) in [
        (4, 1, b"true".as_slice()),
        (5, i64::MIN as u64, b"-9223372036854775808".as_slice()),
        (6, u64::MAX, b"18446744073709551615".as_slice()),
    ] {
        p.argument(argument);
        p.invoke(setter, 0);
        p.source(64);
        p.invoke(31, 0);
        string(&mut p, 64, expected);
    }
    p.invoke(14, 0);
    p.source(64);
    p.invoke(32, 0);
    p.result(0);
    string(&mut p, 64, b"null");
    p.invoke(23, 64);
    passed(p, b"libSceJson", b"libSceJson2");
}

#[test]
fn native_string_destroyed_impl_token_cannot_be_reused_by_a_forged_wrapper() {
    let mut p = Program::new();
    source_text(&mut p);
    p.source(96);
    p.invoke(22, 0);
    p.code
        .extend([0x48, 0x8b, 0x44, 0x24, 0, 0x48, 0x89, 0x44, 0x24, 8]);
    p.invoke(23, 0);
    p.code
        .extend([0x48, 0x8b, 0x44, 0x24, 8, 0x48, 0x89, 0x44, 0x24, 0]);
    p.invoke(25, 0);
    let image = p.finish(b"libSceJson", b"libSceJson2");
    let (status, fields, text) = probe(&image);
    assert_ne!(status, 0, "{text}");
    assert_eq!(fields["code"], "invalid-json-string", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
}

#[test]
fn native_value_assignment_publishes_lazy_source_and_both_destination_spans_together() {
    let mut p = Program::new();
    source_text(&mut p);
    p.source(96);
    p.invoke(27, 0);
    p.invoke(30, 0);
    p.code
        .extend([0x48, 0x89, 0x44, 0x24, 64, 0x48, 0x89, 0xc7]);
    p.call(23); // Destroy the owned wrapper's impl, retaining the wrapper itself.
    p.invoke(0, 32);
    p.store(56, 0x5a5a_5a5a);
    p.source(0);
    p.invoke(13, 32);
    p.pointer(32);
    p.dword(56, 0x5a5a_5a5a);
    p.code.extend([0x48, 0x8b, 0x7c, 0x24, 64]);
    p.call(26);
    p.result(0);
    value_string(&mut p, 0, b"");
    value_string(&mut p, 32, b"");
    p.invoke(14, 0);
    value_string(&mut p, 32, b"");
    p.invoke(14, 32);
    passed(p, b"libSceJson", b"libSceJson2");
}
