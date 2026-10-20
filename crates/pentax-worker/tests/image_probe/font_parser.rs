// SPDX-License-Identifier: GPL-2.0-only
//! Original native Font parsers use real red-zone outputs and actual returns.

use super::*;

const FIRST: i32 = 0x1100;
const SECOND: i32 = 0x1300;
const IMPORTS: [&str; 8] = [
    "oaJ1BpN2FQk",
    "MO24vDhmS4E",
    "Avv7OApgCJk",
    "zN3+nuA0SFQ",
    "mxgmMj-Mq-o",
    "ObkDGDBsVtw",
    "+B-xlbiWDJ4",
    "SSCaczu2aMQ",
];

fn pointer(p: &mut Program, register: u8, offset: u32) {
    p.code.extend([
        0x48 | if register >= 8 { 4 } else { 0 },
        0x8d,
        0x84 | ((register & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn image(code: &[u8], first: &[u8], second: &[u8]) -> Input {
    let original = scoped_system_image(code, &IMPORTS, "Font", "Font", 1);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x1600, 0);
    put(&mut bytes, 96, &0x1600u64.to_le_bytes());
    assert!(0x800 + code.len() < 0xf00);
    assert!(FIRST as usize + first.len() <= SECOND as usize);
    assert!(SECOND as usize + second.len() <= bytes.len());
    put(&mut bytes, FIRST as usize, first);
    put(&mut bytes, SECOND as usize, second);
    create_input(bytes)
}
fn initialize(p: &mut Program, target: Option<i32>) {
    p.set(8, 0); // Actual callback invocation counter.
    pointer(p, 7, 128);
    p.reg(6, 0);
    p.reg(2, 0);
    if let Some(target) = target {
        p.path(1, target);
    } else {
        p.reg(1, u64::MAX);
    }
    p.ptr(8, 8);
    p.call(0);
    p.expect(0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(16);
    p.reg(7, 0);
    pointer(p, 6, 128);
    p.reg(2, 0);
    p.ptr(1, 16);
    p.call(1);
}

fn parser(terminate: bool) -> Vec<u8> {
    let mut code = Vec::new();
    let mut branches = Vec::new();
    // All forty bytes are reset for EACH real invocation; old output must not
    // masquerade as the next parser result. Failure reaches an original UD2.
    code.extend([0x48, 0x83, 0x3e, 0]);
    checked(&mut code, &mut branches);
    for offset in [0, 8, 16, 24] {
        code.extend([0x48, 0x83, 0x7a, offset, 0]);
        checked(&mut code, &mut branches);
    }
    code.extend([
        0x48,
        0x8b,
        0x47,
        40, // MOV RAX,[RDI+40]: retained caller counter.
        0x48,
        0xff,
        0x00, // INC qword[RAX].
        0x48,
        0xc7,
        0x06,
        if terminate { 0x77 } else { 0x55 },
        0,
        0,
        0,
        0xc7,
        0x42,
        8,
        if terminate { 9 } else { b'A' },
        0,
        0,
        0,
    ]);
    if terminate {
        code.extend([0x31, 0xc0, 0xc3]);
    } else {
        // The source rereads text_parser each iteration. The second actual
        // callback therefore has a different immutable executable entry.
        rip(&mut code, FIRST, &[0x48, 0x8d, 0x05], SECOND);
        code.extend([0x48, 0x89, 0x47, 32]);
        // A real nested library call inside the parser preserves its parent
        // callback and the original constructor continuation.
        code.extend([0x48, 0x83, 0xec, 8, 0x31, 0xff]);
        rip(&mut code, FIRST, &[0xff, 0x15], 0xf00 + 5 * 8);
        code.extend([0x48, 0x83, 0xc4, 8, 0xb8, 1, 0, 0, 0, 0xc3]);
    }
    finish_checks(&mut code, branches);
    code
}

#[test]
fn native_font_parser_actual_returns_chain_dynamic_entries_and_keep_original_phase() {
    let mut p = Program::new();
    initialize(&mut p, Some(FIRST));
    p.expect(0);
    p.word(8, 2);
    p.set(24, 0);
    p.load(7, 16);
    p.ptr(6, 24);
    p.call(2);
    p.save(32);
    p.word(24, 1);
    p.set(40, 0);
    p.set(48, 0);
    p.load(7, 32);
    p.ptr(6, 40);
    p.ptr(2, 48);
    p.call(3);
    p.expect(0);
    p.word(40, 0);
    p.word(48, u64::from(b'A'));
    p.load(7, 32);
    p.ptr(6, 56);
    p.call(4);
    p.expect(0);
    p.word(56, 0x55);
    p.load(7, 16);
    p.call(5);
    p.expect(9);
    p.load(7, 16);
    p.call(6);
    p.expect(0x77);
    p.ptr(7, 16);
    p.call(7);
    p.expect(0);
    p.word(16, 0);
    let (code, calls) = p.finish(None);
    let input = image(&code, &parser(false), &parser(true));
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], (calls + 1).to_string(), "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
}

#[test]
fn native_font_parser_errors_and_unreturned_or_unbacked_callbacks_never_publish_empty_success() {
    for kind in 0..4 {
        let mut p = Program::new();
        initialize(&mut p, if kind == 0 { None } else { Some(FIRST) });
        if kind >= 2 {
            p.expect(if kind == 2 {
                u64::from((-57i32) as u32)
            } else {
                u64::from(u32::MAX)
            });
            p.word(16, 0);
        }
        let (code, calls) = p.finish(None);
        let callback = match kind {
            2 => vec![
                0xc7, 0x42, 8, 0xc7, 0xff, 0xff, 0xff, 0xb8, 0xff, 0xff, 0xff, 0xff, 0xc3,
            ],
            3 => vec![0xb8, 2, 0, 0, 0, 0xc3],
            _ => vec![0x0f, 0x0b],
        };
        let input = image(&code, &callback, &[]);
        let (status, fields, text) = probe(&input);
        if kind < 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(
                fields["code"],
                if kind == 0 {
                    "runtime-service"
                } else {
                    "unsupported-instruction"
                },
                "{text}"
            );
            assert!(!fields.contains_key("value"), "{text}");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42", "{text}");
        }
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        let retained_pending = if kind < 2 { "1" } else { "0" };
        assert_eq!(fields["kernel_handles_live"], retained_pending, "{text}");
        assert_eq!(
            fields["kernel_handles_accounted"], retained_pending,
            "{text}"
        );
    }
}
