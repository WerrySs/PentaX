// SPDX-License-Identifier: GPL-2.0-only
//! Original four-byte ACM context handle consumers; no DSP batch is submitted.

use super::*;

const NID: &str = "ZIXln2K3XMk";
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}

fn context_consumer() -> (Vec<u8>, u64) {
    let vector = [
        0x1122_3344_5566_7788,
        0x7ff0_0000_0000_0001,
        0x99aa_bbcc_ddee_ff00,
        0x8000_0000_7fc0_1234,
    ];
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    set_word(&mut p, 0, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for offset in [24, 32, 40, 48, 56, 96, 120] {
        set_word(&mut p, offset, CANARY);
    }
    for (index, value) in vector.into_iter().enumerate() {
        set_word(&mut p, 64 + index as u8 * 8, value);
    }
    p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 64]); // Actual full YMM15 input.
    p.reg(8, 0x8877_6655_4433_2211);
    // These stale nonarguments must not become a guessed parameter structure,
    // callback or second output. The public ABI has exactly one GP argument.
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX);
    p.reg(1, u64::MAX);
    for (offset, first, second) in [
        (32, 1, CANARY),
        (40, 1, 0xa5a5_a5a5_0000_0002),
        (32, 3, 0xa5a5_a5a5_0000_0002),
    ] {
        p.ptr(7, offset);
        p.call(0);
        p.expect(0);
        // Full qword consumers catch a wrong return-only stub, eight-byte store,
        // reused ID or an ID counter reset on every service invocation.
        p.word(32, 0xa5a5_a5a5_0000_0000 | first);
        p.word(40, second);
        p.word(24, CANARY);
        p.word(48, CANARY);
    }
    p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 64]);
    for (index, value) in vector.into_iter().enumerate() {
        p.word(64 + index as u8 * 8, value);
    }
    for offset in [56, 96, 120] {
        p.word(offset, CANARY);
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x1f84);
    p.code.extend([0x4c, 0x89, 0xc0]);
    p.expect(0x8877_6655_4433_2211);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.finish(None)
}

#[test]
fn native_acm_context_creation_binds_exact_scopes_and_writes_sequential_u32_handles() {
    let (code, calls) = context_consumer();
    assert_eq!(calls, 3);
    for scope in ["Acm", "libSceAcm"] {
        let source = scoped_system_image(&code, &[NID], scope, scope, 1);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        // Temporal load/store moves promote under the actual AVX profile;
        // the two MXCSR accesses remain modeled at their exact4-byte widths.
        assert_modeled_vector_counters(&fields, [(4, 72), (2, 8), (2, 8)]);
        assert_eq!(fields["fp_status"], "4", "{text}");
        assert_eq!(fields["integer_ops"], "0", "{text}");
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        assert!(
            fields["kernel_handles_by_type"]
                .split(',')
                .any(|entry| entry == "acm:3"),
            "{text}"
        );
        assert!(
            fields["native_instructions"].parse::<u64>().unwrap() > 0,
            "{text}"
        );
    }

    // Proving each paired scope does not authorize a cross-product of module
    // and library spellings. The consumer has a valid output but must not bind.
    let mut p = Program::new();
    p.ptr(7, 32);
    p.call(0);
    let (code, _) = p.finish(None);
    let source = scoped_system_image(&code, &[NID], "Acm", "libSceAcm", 1);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unavailable-function", "{text}");
    assert_eq!(fields["reference_match"], "different-scope", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
}
