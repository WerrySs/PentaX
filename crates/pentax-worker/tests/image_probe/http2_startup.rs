// SPDX-License-Identifier: GPL-2.0-only

//! Original native consumer of all twenty-one HTTP2 metadata exports.

use super::*;

const IMPORTS: [&str; 22] = [
    "3JCe3lCbQ8A", // Http2Init
    "YiBUtz-pGkc", // Http2Term
    "+wCt7fCijgk", // Http2CreateTemplate
    "pDom5-078DA", // Http2DeleteTemplate
    "mmyOCxQMVYQ", // Http2CreateRequestWithURL
    "c8D9qIjo8EY", // Http2DeleteRequest
    "nrPfOE8TQu0", // Http2AddRequestHeader
    "FSAFOzi0FpM", // Http2SetRequestContentLength
    "jjFahkBPCYs", // Http2SetAuthEnabled
    "b9AvoIaOuHI", // Http2SetAutoRedirect
    "uRosf8GQbHQ", // Http2SetInflateGZIPEnabled
    "B37SruheQ5Y", // Http2SslDisableOption
    "EWcwMpbr5F8", // Http2SslEnableOption
    "BJgi0CH7al4", // Http2SetRedirectCallback
    "YrWX+DhPHQY", // Http2SetSslCallback
    "izvHhqgDt44", // Http2SetRecvTimeOut
    "XPtW45xiLHk", // Http2SetSendTimeOut
    "-HIO4VT87v8", // Http2SetConnectTimeOut
    "n8hMLe31OPA", // Http2SetConnectionWaitTimeOut
    "ACjtE27aErY", // Http2SetResolveTimeOut
    "VYMxTcBqSE0", // Http2SetTimeOut
    "HQOwnfMGipQ", // Separate Net errno location, never a transport call.
];
const INVALID: u64 = 0x817b_1100;
const NULL: u64 = 0x817b_1225;
const METHOD: i32 = 0x1300;
const URL: i32 = 0x1310;
const HEADER: i32 = 0x1320;
const VALUE: i32 = 0x1330;

fn http2_image(code: &[u8]) -> Input {
    const STRINGS: usize = 0x1400;
    const SYMBOLS: usize = 0x1c00;
    const RELOCATIONS: usize = 0x2200;
    assert!(0x800 + code.len() <= 0xf00);
    assert!(0xf00 + IMPORTS.len() * 8 < METHOD as usize);
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x2800, 0);
    bytes[0x200..0x800].fill(0);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    let file_size = bytes.len() as u64;
    put(&mut bytes, 96, &file_size.to_le_bytes());
    put(&mut bytes, 0x800, code);
    put(&mut bytes, METHOD as usize, &[0xff, 0x80, 0]);
    put(&mut bytes, URL as usize, &[0xfe, b':', b'/', b'/', 0x81, 0]);
    put(&mut bytes, HEADER as usize, &[0xfd, 0x82, 0]);
    put(&mut bytes, VALUE as usize, &[0xfc, 0x83, 0]);

    let mut names = vec![0];
    let mut scopes = Vec::new();
    for name in ["libSceHttp2", "libSceNet"] {
        let module = names.len() as u64;
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        let library = names.len() as u64;
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        scopes.push((module, library));
    }
    for (index, nid) in IMPORTS.iter().enumerate() {
        let scope = usize::from(index == 21);
        let module_id = char::from(b'C' + scope as u8 * 2);
        let library_id = char::from(b'D' + scope as u8 * 2);
        let symbol = SYMBOLS + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12; // Undefined GLOBAL FUNC.
        names.extend_from_slice(format!("{nid}#{library_id}#{module_id}\0").as_bytes());
        put(
            &mut bytes,
            RELOCATIONS + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            RELOCATIONS + index * 24 + 8,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(STRINGS + names.len() <= SYMBOLS);
    assert!(SYMBOLS + (IMPORTS.len() + 1) * 24 <= RELOCATIONS);
    assert!(RELOCATIONS + IMPORTS.len() * 24 <= bytes.len());
    put(&mut bytes, STRINGS, &names);
    let mut dynamic = vec![
        (5_i64, STRINGS as u64),
        (10, names.len() as u64),
        (6, SYMBOLS as u64),
        (39, (IMPORTS.len() as u64 + 1) * 24),
        (11, 24),
    ];
    for (index, (module, library)) in scopes.into_iter().enumerate() {
        let module_id = 2 + index as u64 * 2;
        dynamic.push((
            0x6100_000f,
            module | (1 << 32) | (1 << 40) | (module_id << 48),
        ));
        dynamic.push((0x6100_0015, library | (1 << 32) | ((module_id + 1) << 48)));
    }
    dynamic.extend([
        (7, RELOCATIONS as u64),
        (8, IMPORTS.len() as u64 * 24),
        (9, 24),
        (0, 0),
    ]);
    assert!(0x200 + dynamic.len() * 16 < 0x800);
    for offset in [152, 160] {
        put(
            &mut bytes,
            offset,
            &((dynamic.len() * 16) as u64).to_le_bytes(),
        );
    }
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}

fn int(p: &mut Program, register: u8, value: u32) {
    assert!(register < 8);
    p.code.push(0xb8 + register); // MOV r32,imm32, including zero extension.
    p.code.extend(value.to_le_bytes());
}

fn zero(p: &mut Program) {
    p.code.extend([0x48, 0x85, 0xc0]); // Check the complete returned RAX.
    checked(&mut p.code, &mut p.branches);
}

fn one(p: &mut Program, slot: i32, id: u32, expected: u64) {
    int(p, 7, id);
    p.call(slot);
    if expected == 0 {
        zero(p);
    } else {
        p.expect(expected);
    }
}

fn context(p: &mut Program, expected: u64) {
    p.reg(7, u64::MAX); // Invalid signed32 dependency remains metadata.
    p.reg(6, 0x1234_5678_8765_4321);
    int(p, 2, 0); // Source accepts a zero pool size.
    p.reg(1, u64::MAX); // Negative signed32 concurrency is retained.
    p.call(0);
    p.expect(expected);
}

fn template(p: &mut Program, context: u32, raw_agent: bool, expected: u64) {
    int(p, 7, context);
    if raw_agent {
        p.path(6, METHOD);
    } else {
        int(p, 6, 0); // Nullable user agent becomes an empty owned string.
    }
    int(p, 2, u32::MAX);
    p.reg(1, 0xaabb_ccdd_8000_0000);
    p.call(2);
    p.expect(expected);
}

fn request(p: &mut Program, template: u32, expected: u64) {
    int(p, 7, template);
    p.path(6, METHOD);
    p.path(2, URL);
    p.reg(1, u64::MAX);
    p.call(4);
    p.expect(expected);
}

#[test]
fn native_http2_metadata_exercises_every_export_and_real_cascading_cleanup() {
    let mut p = Program::new();
    p.call(21);
    p.save(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x8b, 0x03]); // Initially zero independent Net errno.
    zero(&mut p);
    p.load(11, 0);
    p.code.extend([0x41, 0xc7, 0x03, 77, 0, 0, 0]);
    p.reg(0, 0x1937_6284_55aa_ccdd);
    p.save(64); // No metadata call has a guest output span here.

    context(&mut p, 1);
    template(&mut p, 1, false, 1);
    int(&mut p, 7, u32::MAX);
    int(&mut p, 6, 0);
    int(&mut p, 2, 0);
    p.call(4);
    p.expect(NULL); // Required null strings precede even an invalid ID.
    request(&mut p, 1, 1); // All three independent ID spaces now collide at1.

    p.reg(7, 0xaaaa_bbbb_0000_0001); // Request identity consumes only low32.
    p.path(6, HEADER);
    p.path(2, VALUE);
    p.reg(1, u64::MAX); // Mode is ignored; opaque bytes are not UTF-8.
    p.call(6);
    zero(&mut p);
    int(&mut p, 7, 1);
    p.reg(6, u64::MAX);
    p.call(7);
    zero(&mut p);
    for slot in 8..=10 {
        int(&mut p, 7, 1);
        int(&mut p, 6, 0x8000_0000); // Every nonzero signed32 value enables.
        p.call(slot);
        zero(&mut p);
    }
    for slot in 11..=12 {
        int(&mut p, 7, 1);
        int(&mut p, 6, 0x8765_4321);
        p.call(slot);
        zero(&mut p);
    }
    for slot in 13..=14 {
        int(&mut p, 7, 1);
        p.reg(6, 0x8000_0000_0000_0001); // Noncanonical opaque callback word.
        p.reg(2, 0x8000_0000_0000_0003);
        p.call(slot);
        zero(&mut p); // Completion requires that neither word was executed.
    }
    for slot in 15..=20 {
        int(&mut p, 7, 1);
        int(&mut p, 6, u32::MAX);
        p.call(slot);
        zero(&mut p);
    }

    context(&mut p, 2);
    int(&mut p, 7, 2);
    int(&mut p, 6, 1);
    p.call(8);
    p.expect(INVALID); // A context alone is never an options owner.
    template(&mut p, 2, true, 2);
    request(&mut p, 2, 2);
    one(&mut p, 5, 2, 0);
    request(&mut p, 2, 3); // Request IDs do not reuse the deleted ordinal2.
    one(&mut p, 3, 2, 0); // Explicit template deletion cascades into request3.
    one(&mut p, 5, 3, INVALID);
    template(&mut p, 2, true, 3); // Template IDs do not reuse ordinal2 either.
    request(&mut p, 3, 4);
    one(&mut p, 1, 1, 0); // Context1 deletes the original template1/request1.
    one(&mut p, 3, 1, INVALID);
    one(&mut p, 5, 1, INVALID);
    one(&mut p, 1, 2, 0); // Context2 deletes template3/request4.
    one(&mut p, 3, 3, INVALID);
    one(&mut p, 5, 4, INVALID);
    one(&mut p, 1, 2, INVALID);
    context(&mut p, 3); // Term does not reset the context identity counter.
    one(&mut p, 1, 3, 0);

    p.load(11, 0);
    p.code.extend([0x41, 0x8b, 0x03]);
    p.expect(77); // Neither successful nor failing HTTP2 calls touch Net errno.
    p.word(64, 0x1937_6284_55aa_ccdd);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    // This fixture uses more code than the shared compact Program::finish.
    // Its own bound keeps every byte before the GOT, raw strings and tables.
    assert!(p.code.len() < 0x700);
    let input = http2_image(&p.code);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["service_stops"], p.calls.to_string(), "{text}");
    assert_eq!(fields["output_bytes"], "0", "{text}");
    assert_eq!(fields["kernel_handles_live"], "1", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "1", "{text}");
    assert!(
        fields["kernel_handles_by_type"].contains("memory:1,"),
        "{text}"
    );
    assert!(
        fields["kernel_handles_by_type"].contains("network:0,"),
        "{text}"
    );
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}
