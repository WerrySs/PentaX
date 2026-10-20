// SPDX-License-Identifier: GPL-2.0-only
//! Original native parser, owning containers and actual guest iterator cells.
use super::json_primitives::Program;
use super::*;

const FUNCTIONS: [&[u8; 11]; 22] = [
    b"OJPTonqdg0I",
    b"a+W7HHlwpBs",
    b"5JmzZt8twAo",
    b"ERuf9y0DY84",
    b"JP-PtKMiI1E",
    b"HJ8GpRT1aiw",
    b"zQtLRTqceMY",
    b"3xUXnmUkXfo",
    b"iZeYfOxtMRg",
    b"dFCphqnd+a4",
    b"IlsmvBtMkak",
    b"ONT8As5R1ug",
    b"HwDt5lD9Bfo",
    b"XlWbvieLj2M",
    b"wLsJlmgEIaI",
    b"bcH5EnFE2xY",
    b"WXF2ihRF+B8",
    b"5AZPp99ogrc",
    b"wcgr5mte7T8",
    b"w5+VCznos5E",
    b"9yLjn46Ypfs",
    b"S5JxQnoGF3E",
];
fn call(p: &mut Program, calls: &mut u64, nid: &'static [u8; 11]) {
    p.call_nid(nid);
    *calls += 1;
}
fn pointer(p: &mut Program, reg: u8, offset: u32) {
    p.code.extend([
        0x48 | if reg >= 8 { 4 } else { 0 },
        0x8d,
        0x84 | ((reg & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn load(p: &mut Program, reg: u8, offset: u32) {
    p.code.extend([
        0x48 | if reg >= 8 { 4 } else { 0 },
        0x8b,
        0x84 | ((reg & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn save(p: &mut Program, offset: u32) {
    p.code.extend([0x48, 0x89, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn data(p: &mut Program, bytes: &[u8]) {
    assert!(bytes.len() < 96);
    for (index, chunk) in bytes.chunks(8).enumerate() {
        let mut word = [0; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        p.code.extend([0x48, 0xb8]);
        p.code.extend(word);
        save(p, 160 + index as u32 * 8);
    }
}
fn integer(p: &mut Program, calls: &mut u64, expected: u64) {
    p.code.extend([0x48, 0x89, 0xc7]); // returned Value pointer, not a scalar result
    call(p, calls, b"DIxvoy7Ngvk");
    p.code.extend([0x48, 0x8b, 0x00]);
    p.result(expected);
}
fn finish(p: Program, calls: u64, module: &[u8], library: &[u8]) -> (Input, u64) {
    (p.finish(module, library), calls)
}
fn parsed(module: &[u8], library: &[u8]) -> (Input, u64) {
    let mut p = Program::new();
    let mut calls = 0;
    let json = br#"{"a":[7,-2],"o":{"v":13},"r":1.25}"#;
    data(&mut p, json);
    p.object(0);
    pointer(&mut p, 6, 160);
    p.code.extend([0x48, 0xba]);
    p.code.extend((json.len() as u64).to_le_bytes());
    call(&mut p, &mut calls, FUNCTIONS[21]);
    p.result(0);
    data(&mut p, b"a\0");
    p.object(0);
    pointer(&mut p, 6, 160);
    call(&mut p, &mut calls, FUNCTIONS[12]);
    save(&mut p, 96);
    p.code.extend([0x48, 0x89, 0xc7]);
    call(&mut p, &mut calls, FUNCTIONS[11]);
    save(&mut p, 104);
    for (function, output) in [(15, 64), (16, 72)] {
        p.object(output);
        load(&mut p, 6, 104);
        call(&mut p, &mut calls, FUNCTIONS[function]);
        p.pointer(output);
    }
    for expected in [7, (-2_i64) as u64] {
        p.object(64);
        p.source(72);
        call(&mut p, &mut calls, FUNCTIONS[17]);
        p.result(1);
        p.object(64);
        call(&mut p, &mut calls, FUNCTIONS[18]);
        integer(&mut p, &mut calls, expected);
        p.object(64);
        call(&mut p, &mut calls, FUNCTIONS[19]);
        p.pointer(64);
    }
    p.object(64);
    p.source(72);
    call(&mut p, &mut calls, FUNCTIONS[17]);
    p.result(0);
    p.code.extend([0x48, 0xbf]);
    p.code.extend(u64::MAX.to_le_bytes());
    call(&mut p, &mut calls, FUNCTIONS[20]);
    load(&mut p, 7, 96);
    p.argument(1);
    call(&mut p, &mut calls, FUNCTIONS[13]);
    integer(&mut p, &mut calls, (-2_i64) as u64);
    data(&mut p, b"o\0");
    p.object(0);
    pointer(&mut p, 6, 160);
    call(&mut p, &mut calls, FUNCTIONS[12]);
    save(&mut p, 120);
    p.code.extend([0x48, 0x89, 0xc7]);
    call(&mut p, &mut calls, FUNCTIONS[10]);
    save(&mut p, 112);
    data(&mut p, b"v\0");
    p.object(80);
    pointer(&mut p, 6, 160);
    call(&mut p, &mut calls, b"9KUZFjI1IxA");
    load(&mut p, 7, 120);
    p.source(80);
    call(&mut p, &mut calls, FUNCTIONS[14]);
    integer(&mut p, &mut calls, 13);
    p.object(32);
    p.source(0);
    call(&mut p, &mut calls, b"fSb2oQTNrgA");
    p.pointer(32);
    p.object(0);
    call(&mut p, &mut calls, b"WTtYf+cNnXI");
    p.object(88);
    call(&mut p, &mut calls, b"qSmqLXXCPas");
    p.object(32);
    p.source(88);
    call(&mut p, &mut calls, b"R7FDWtcN6f8");
    p.result(0);
    let expected = b"{\"a\":[7,-2],\"o\":{\"v\":13},\"r\":1.250000}\0";
    p.object(88);
    call(&mut p, &mut calls, b"EUH+EmT-v9E");
    p.result((expected.len() - 1) as u64);
    p.object(88);
    call(&mut p, &mut calls, b"L1KAkYWml-M");
    save(&mut p, 112);
    for (index, chunk) in expected.chunks(8).enumerate() {
        load(&mut p, 11, 112);
        if chunk.len() == 8 {
            p.code.extend([0x49, 0x8b, 0x43, index as u8 * 8]);
            p.result(u64::from_le_bytes(chunk.try_into().unwrap()));
        } else {
            for (byte, expected) in chunk.iter().enumerate() {
                p.code
                    .extend([0x41, 0x0f, 0xb6, 0x43, (index * 8 + byte) as u8]);
                p.result(u64::from(*expected));
            }
        }
    }
    finish(p, calls, module, library)
}
fn lifetimes(module: &[u8], library: &[u8]) -> (Input, u64) {
    let mut p = Program::new();
    let mut calls = 0;
    data(&mut p, b"key\0");
    p.object(88);
    pointer(&mut p, 6, 160);
    call(&mut p, &mut calls, b"9KUZFjI1IxA");
    p.object(64);
    call(&mut p, &mut calls, FUNCTIONS[0]);
    p.pointer(64);
    p.object(64);
    p.source(88);
    call(&mut p, &mut calls, FUNCTIONS[3]);
    save(&mut p, 136);
    p.object(0);
    p.argument(42);
    call(&mut p, &mut calls, b"0lLK8+kDqmE");
    p.pointer(0);
    load(&mut p, 7, 136);
    p.source(0);
    call(&mut p, &mut calls, b"4zrm6VrgIAw");
    p.object(72);
    p.source(64);
    call(&mut p, &mut calls, FUNCTIONS[1]);
    p.pointer(72);
    p.object(64);
    call(&mut p, &mut calls, FUNCTIONS[2]);
    p.word(64, 0);
    p.object(72);
    p.source(88);
    call(&mut p, &mut calls, FUNCTIONS[3]);
    integer(&mut p, &mut calls, 42);
    p.object(32);
    p.source(72);
    call(&mut p, &mut calls, FUNCTIONS[7]);
    p.pointer(32);
    p.object(32);
    call(&mut p, &mut calls, FUNCTIONS[10]);
    save(&mut p, 144);
    p.object(32);
    load(&mut p, 6, 144);
    call(&mut p, &mut calls, FUNCTIONS[9]);
    p.result(0);
    p.object(80);
    call(&mut p, &mut calls, FUNCTIONS[4]);
    p.pointer(80);
    p.object(80);
    p.source(32);
    call(&mut p, &mut calls, FUNCTIONS[6]);
    p.pointer(80);
    p.object(96);
    p.source(80);
    call(&mut p, &mut calls, FUNCTIONS[8]);
    p.pointer(96);
    p.object(96);
    call(&mut p, &mut calls, b"RBw+4NukeGQ");
    p.result(1);
    p.object(80);
    call(&mut p, &mut calls, FUNCTIONS[5]);
    p.word(80, 0);
    p.object(96);
    p.argument(0);
    call(&mut p, &mut calls, FUNCTIONS[13]);
    p.code.extend([0x48, 0x89, 0xc7]);
    call(&mut p, &mut calls, FUNCTIONS[10]);
    p.code.extend([0x48, 0x89, 0xc7]);
    p.source(88);
    call(&mut p, &mut calls, FUNCTIONS[3]);
    integer(&mut p, &mut calls, 42);
    p.object(72);
    call(&mut p, &mut calls, FUNCTIONS[2]);
    p.word(72, 0);
    for value in [32, 96] {
        p.object(value);
        call(&mut p, &mut calls, b"WTtYf+cNnXI");
        p.word(value + 16, 0);
    }
    finish(p, calls, module, library)
}

#[test]
fn native_json_trees_parse_copy_iterate_serialize_and_destroy_through_exact_imports() {
    for (module, library) in [
        (b"Json2".as_slice(), b"Json2".as_slice()),
        (b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
    ] {
        for (image, calls) in [parsed(module, library), lifetimes(module, library)] {
            let (status, fields, text) = probe(&image);
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42", "{text}");
            assert_eq!(fields["unavailable_functions"], "0", "{text}");
            assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        }
    }
}
