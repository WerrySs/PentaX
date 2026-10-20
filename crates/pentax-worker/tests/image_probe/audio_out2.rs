// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers of four stateless AudioOut2 contracts; no device is opened.

use super::*;

const IMPORTS: [&str; 4] = ["g2tViFIohHE", "t5YrizufpQc", "pDmme7Bgm6E", "DImz2Ft9E2g"];
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn fill_stack(p: &mut Program) {
    p.reg(0, CANARY);
    for offset in (8..=120).step_by(8) {
        p.save(offset);
    }
}

fn store(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}

fn consumer() -> (Vec<u8>, u64) {
    let mut p = Program::new();
    p.stack(0x89, 4, 0); // Retain the actual caller stack pointer.
    p.reg(8, 0x8877_6655_4433_2211);
    fill_stack(&mut p);
    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX);
    p.call(0); // Initialize has no arguments and no input-memory contract.
    p.expect(0);

    p.ptr(7, 32);
    p.call(1);
    p.expect(0);
    for (offset, expected) in [
        (24, CANARY),
        (32, 0x0000_0100_0000_0100),
        (40, 0x0000_0004_0000_0000),
        (48, 0x0000_0001_0000_0200),
        (56, 0),
        (64, 0),
        (72, 0),
        (80, 0),
        (88, 0),
        (96, CANARY),
    ] {
        p.word(offset, expected);
    }

    p.ptr(7, 32);
    p.ptr(6, 104);
    p.call(2);
    p.expect(0);
    p.word(104, 0x1_1640); // Default queue_depth4 from the real reset result.
    p.word(96, CANARY);
    p.word(112, CANARY);

    // Only queue_depth, the second dword of this qword, affects sizing.
    store(&mut p, 40, 0x0000_0007_7654_3210);
    p.ptr(7, 32);
    p.ptr(6, 104);
    p.call(2);
    p.expect(0);
    p.word(104, 0x1_26f0);
    p.word(40, 0x0000_0007_7654_3210);
    store(&mut p, 40, 0x0000_0000_7654_3210);
    p.ptr(7, 32);
    p.ptr(6, 104);
    p.call(2);
    p.expect(0);
    p.word(104, 0x1_1640); // Explicit zero queue_depth also means four.

    // The eight-byte output deliberately covers the four-byte input field.
    // Reading after a premature output clear would produce the default size;
    // truncating the source size_t to u32 would lose the high result bits.
    store(&mut p, 40, u64::MAX);
    p.ptr(7, 32);
    p.ptr(6, 40);
    p.call(2);
    p.expect(0);
    p.word(40, 0x1_0000 + u64::from(u32::MAX) * 0x590);
    p.word(32, 0x0000_0100_0000_0100);
    p.word(48, 0x0000_0001_0000_0200);
    p.word(24, CANARY);
    p.word(96, CANARY);

    fill_stack(&mut p);
    p.ptr(7, 16);
    p.reg(6, u64::MAX); // Speaker-info flags are ignored, not a guest pointer.
    p.call(3);
    p.expect(0);
    for (offset, expected) in [
        (8, CANARY),
        (16, 0x0000_0003_0000_0000),
        (24, 0),
        (32, 0x0000_001e_0000_ffe2), // Signed i16 azimuths -30 and +30.
        (40, 0),
        (48, 0),
        (56, 0),
        (64, 0),
        (72, 0),
        (80, 0),
        (88, 0),
        (96, CANARY),
    ] {
        p.word(offset, expected);
    }
    p.reg(7, u64::MAX);
    p.call(0); // Repeating Init neither resets these outputs nor creates state.
    p.expect(0);
    p.word(16, 0x0000_0003_0000_0000);
    p.word(32, 0x0000_001e_0000_ffe2);
    p.word(96, CANARY);
    p.code.extend([0x4c, 0x89, 0xc0]);
    p.expect(0x8877_6655_4433_2211);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 0);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700);
    (p.code, p.calls)
}

fn input(code: &[u8], module: &str, library: &str) -> Input {
    let mut source = scoped_system_image(code, &IMPORTS, module, library, 1);
    // The longer original consumer also occupies the helper's unused fixed
    // path-name window. Restore its own code without touching the GOT at0xf00.
    assert!(code.len() < 0x700);
    put(&mut source.bytes, 0x800, code);
    std::fs::write(&source.path, &source.bytes).unwrap();
    source
}

#[test]
fn native_audio_out2_stateless_profiles_preserve_exact_sizes_aliases_and_paired_scopes() {
    let (code, calls) = consumer();
    assert_eq!(calls, 8);
    for (module, library) in [
        ("AudioOut", "AudioOut2"),
        ("libSceAudioOut", "libSceAudioOut2"),
    ] {
        let source = input(&code, module, library);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
        assert_eq!(fields["vector_ops"], "0", "{text}");
        assert_eq!(fields["vector_bytes"], "0", "{text}");
        assert!(
            fields["native_instructions"].parse::<u64>().unwrap() > 0,
            "{text}"
        );
    }

    // Two individually admitted pairs do not authorize their cross product.
    let mut p = Program::new();
    p.call(0);
    let (code, _) = p.finish(None);
    for (module, library) in [
        ("AudioOut", "libSceAudioOut2"),
        ("libSceAudioOut", "AudioOut2"),
    ] {
        let source = input(&code, module, library);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "unavailable-function", "{text}");
        assert_eq!(fields["reference_match"], "different-scope", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}
