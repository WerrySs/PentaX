// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_content_integer_query_preserves_source_sku_absent_metadata_and_four_byte_outputs() {
    let mut p = Program::new();
    for id in 0..=4 {
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        for offset in [8, 16, 24] {
            p.save(offset);
        }
        p.reg(7, (7 << 32) | id);
        p.ptr(6, 16);
        p.call(0);
        p.expect(0);
        p.word(8, 0xa5a5_a5a5_a5a5_a5a5);
        p.word(16, 0xa5a5_a5a5_0000_0000 | if id == 0 { 3 } else { 0 });
        p.word(24, 0xa5a5_a5a5_a5a5_a5a5);
    }
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &["99b82IKXpH4"],
        "libSceAppContentUtil",
        "libSceAppContent",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

fn address(p: &mut Program, reg: u8, offset: i32) {
    p.code.extend([
        0x48 | if reg >= 8 { 4 } else { 0 },
        0x8d,
        0x84 | ((reg & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn word(p: &mut Program, offset: i32, expected: u64) {
    address(p, 11, offset);
    p.code.extend([0x49, 0x8b, 0x03]);
    p.expect(expected);
}
fn mixed_image(code: &[u8], module: &str, library: &str, version: u64) -> Input {
    let mut bytes = module_bytes(&[], true, None);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    put(&mut bytes, 0x800, code);
    let mut names = vec![0];
    let mut offsets = Vec::new();
    for text in [
        module,
        library,
        "libSceSystemService",
        "libSceSystemService",
    ] {
        offsets.push(names.len() as u64);
        names.extend(text.as_bytes());
        names.push(0);
    }
    bytes[0x600..0x800].fill(0);
    for (i, name) in ["R9lA82OraNs#D#C", "656LMQSrg6U#F#E", "rPo6tV8D9bM#F#E"]
        .iter()
        .enumerate()
    {
        let at = 0x600 + (i + 1) * 24;
        put(&mut bytes, at, &(names.len() as u32).to_le_bytes());
        bytes[at + 4] = 0x12;
        names.extend(name.as_bytes());
        names.push(0);
        put(
            &mut bytes,
            0x700 + i * 24,
            &(0xf00_u64 + i as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x708 + i * 24,
            &(((i as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    put(&mut bytes, 0x400, &names);
    for (i, (tag, value)) in [
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, 4 * 24),
        (11, 24),
        (0x6100_000f, offsets[0] | (1 << 32) | (1 << 40) | (2 << 48)),
        (0x6100_0015, offsets[1] | (version << 32) | (3 << 48)),
        (7, 0x700),
        (8, 3 * 24),
        (9, 24),
        (0x6100_000f, offsets[2] | (1 << 32) | (1 << 40) | (4 << 48)),
        (0x6100_0015, offsets[3] | (1 << 32) | (5 << 48)),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + i * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + i * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(13_u64 * 16).to_le_bytes());
    }
    create_input(bytes)
}

#[test]
fn native_content_initialization_and_system_events_share_real_process_state_across_scopes() {
    let mut p = Program::new();
    p.code.extend([0x48, 0x81, 0xec, 0, 0x30, 0, 0]); // Private event-output space.
    p.reg(0, u64::MAX);
    p.save(40);
    p.reg(7, u64::MAX); // Ignored source init_param is never dereferenced.
    p.ptr(6, 0);
    p.call(0);
    p.expect(0);
    for at in [0, 8, 16, 24, 32] {
        p.word(at, 0);
    }
    p.word(40, u64::MAX);
    p.reg(6, 0);
    p.call(0);
    p.expect(0);
    p.ptr(7, 64);
    p.call(2);
    p.expect(0);
    p.word(64, 2);
    for remaining in [1, 0] {
        address(&mut p, 7, 256);
        p.reg(0, 0xa5);
        p.reg(1, 8204);
        p.code.extend([0xf3, 0xaa]);
        address(&mut p, 7, 256);
        p.call(1);
        p.expect(0);
        word(&mut p, 256, 0x1000_0003);
        word(&mut p, 8444, 0);
        word(&mut p, 8452, 0xa5a5_a5a5_a5a5_a5a5);
        p.ptr(7, 64);
        p.call(2);
        p.expect(0);
        p.word(64, remaining);
    }
    address(&mut p, 7, 256);
    p.call(1);
    p.expect(0x80a1_0004);
    word(&mut p, 256, 0x0000_0000_ffff_ffff);
    for slot in [1, 2] {
        p.reg(7, 0);
        p.call(slot);
        p.expect(0x80a1_0003);
    }
    p.code.extend([0x48, 0x81, 0xc4, 0, 0x30, 0, 0]);
    let (code, calls) = p.finish(None);
    for (module, library, version, success) in [
        ("AppContentUtil", "AppContent", 1, true),
        ("libSceAppContentUtil", "AppContent", 1, true),
        ("AppContentUtil", "libSceAppContent", 1, true),
        ("libSceAppContentUtil", "libSceAppContent", 1, true),
        ("libSceAppContentUtil", "libSceAppContent", 2, false),
    ] {
        let input = mixed_image(&code, module, library, version);
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
