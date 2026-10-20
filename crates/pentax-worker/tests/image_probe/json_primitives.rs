// SPDX-License-Identifier: GPL-2.0-only

//! Original imported native scalar lifecycle and reference-return ABI checks.
use super::*;

#[path = "json_strings.rs"]
mod json_strings;

const NIDS: [&[u8; 11]; 33] = [
    b"qBMjqyBn3OM",
    b"UeuWT+yNdCQ",
    b"0lLK8+kDqmE",
    b"CbrT3dwDILo",
    b"5yHuiWXo2gg",
    b"QxVVYhP-mvg",
    b"SIe1ZmW7e7s",
    b"IKQimvG9Wqs",
    b"SHtAad20YYM",
    b"zTwZdI8AZ5Y",
    b"DIxvoy7Ngvk",
    b"3qrge7L-AU4",
    b"fSb2oQTNrgA",
    b"4zrm6VrgIAw",
    b"WTtYf+cNnXI",
    b"FIjXN2TkuTs",
    b"RBw+4NukeGQ",
    b"-hJRce8wn1U",
    b"OcAgPxcq5Vk",
    b"sOmU4vnx3s0",
    b"BSmWDIkV4w4",
    b"qSmqLXXCPas",
    b"9KUZFjI1IxA",
    b"cG1VE2HMl6c",
    b"cn9svYGWKDQ",
    b"L1KAkYWml-M",
    b"EUH+EmT-v9E",
    b"b9V6fmppLXY",
    b"sZIoMRGO+jk",
    b"6l3Bv2gysNc",
    b"epJ6x2LV0kU",
    b"Ncel8t2Rrpc",
    b"R7FDWtcN6f8",
];

pub(super) struct Program {
    pub(super) code: Vec<u8>,
    branches: Vec<usize>,
    imports: Vec<&'static [u8; 11]>,
}
impl Program {
    pub(super) fn new() -> Self {
        Self {
            code: vec![0x55, 0x48, 0x81, 0xec, 0, 1, 0, 0],
            branches: Vec::new(),
            imports: Vec::new(),
        }
    }
    fn call(&mut self, function: usize) {
        self.call_nid(NIDS[function]);
    }
    pub(super) fn call_nid(&mut self, nid: &'static [u8; 11]) {
        let index = match self.imports.iter().position(|&item| item == nid) {
            Some(index) => index,
            None => {
                self.imports.push(nid);
                self.imports.len() - 1
            }
        };
        rip(
            &mut self.code,
            0x100,
            &[0xff, 0x15],
            0x3000 + index as i32 * 8,
        );
    }
    pub(super) fn object(&mut self, offset: u8) {
        self.code.extend([0x48, 0x8d, 0x7c, 0x24, offset]);
    }
    pub(super) fn source(&mut self, offset: u8) {
        self.code.extend([0x48, 0x8d, 0x74, 0x24, offset]);
    }
    pub(super) fn argument(&mut self, value: u64) {
        self.code.extend([0x48, 0xbe]);
        self.code.extend(value.to_le_bytes());
    }
    fn invoke(&mut self, function: usize, object: u8) {
        self.object(object);
        self.call(function);
    }
    pub(super) fn checked(&mut self) {
        checked(&mut self.code, &mut self.branches);
    }
    pub(super) fn result(&mut self, value: u64) {
        self.code.extend([0x48, 0xba]);
        self.code.extend(value.to_le_bytes());
        self.code.extend([0x48, 0x39, 0xd0]);
        self.checked();
    }
    pub(super) fn pointer(&mut self, offset: u8) {
        self.code
            .extend([0x48, 0x8d, 0x54, 0x24, offset, 0x48, 0x39, 0xd0]);
        self.checked();
    }
    pub(super) fn store(&mut self, offset: u8, value: u64) {
        self.code.extend([0x48, 0xb8]);
        self.code.extend(value.to_le_bytes());
        self.code.extend([0x48, 0x89, 0x44, 0x24, offset]);
    }
    fn real_argument(&mut self, bits: u64) {
        self.store(96, bits);
        self.store(104, 0x0123_4567_89ab_cdef);
        // MOVDQU xmm0,[rsp+96]: the captured argument comes from guest SSE,
        // with an unrelated RSI and a distinct upper-half canary.
        self.code.extend([0xf3, 0x0f, 0x6f, 0x44, 0x24, 96]);
        self.argument(!bits);
    }
    fn preserved_real_argument(&mut self, bits: u64) {
        self.code.extend([0xf3, 0x0f, 0x7f, 0x44, 0x24, 64]);
        self.word(64, bits);
        self.word(72, 0x0123_4567_89ab_cdef);
    }
    pub(super) fn word(&mut self, offset: u8, expected: u64) {
        self.code.extend([0x48, 0x8b, 0x44, 0x24, offset]);
        self.result(expected);
    }
    pub(super) fn dword(&mut self, offset: u8, expected: u64) {
        self.code.extend([0x8b, 0x44, 0x24, offset]);
        self.result(expected);
    }
    pub(super) fn finish(mut self, module: &[u8], library: &[u8]) -> Input {
        self.code
            .extend([0xb8, 42, 0, 0, 0, 0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.code, self.branches);
        assert!(self.code.len() < 0xf00);
        let mut image = vec![0; 0x3100];
        put(&mut image, 0, b"\x7fELF\x02\x01\x01");
        for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 2)] {
            put(&mut image, at, &value.to_le_bytes());
        }
        put(&mut image, 20, &1_u32.to_le_bytes());
        put(&mut image, 24, &0x100_u64.to_le_bytes());
        put(&mut image, 32, &64_u64.to_le_bytes());
        for (index, kind, flags, offset, size, span, align) in [
            (
                0,
                1_u32,
                5_u32,
                0_u64,
                image.len() as u64,
                0x4000_u64,
                0x4000_u64,
            ),
            (1, 2, 4, 0x1000, 11 * 16, 11 * 16, 8),
        ] {
            let at = 64 + index * 56;
            put(&mut image, at, &kind.to_le_bytes());
            put(&mut image, at + 4, &flags.to_le_bytes());
            for (field, value) in [
                (8, offset),
                (16, offset),
                (32, size),
                (40, span),
                (48, align),
            ] {
                put(&mut image, at + field, &value.to_le_bytes());
            }
        }
        put(&mut image, 0x100, &self.code);
        let mut names = vec![0];
        names.extend_from_slice(module);
        names.push(0);
        let library_name = names.len() as u64;
        names.extend_from_slice(library);
        names.push(0);
        for (index, nid) in self.imports.iter().enumerate() {
            let name = names.len() as u32;
            names.extend_from_slice(*nid);
            names.extend_from_slice(b"#D#C\0");
            let symbol = 0x1800 + (index + 1) * 24;
            put(&mut image, symbol, &name.to_le_bytes());
            image[symbol + 4] = 0x12;
            put(
                &mut image,
                0x2000 + index * 24,
                &(0x3000_u64 + index as u64 * 8).to_le_bytes(),
            );
            put(
                &mut image,
                0x2008 + index * 24,
                &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
            );
        }
        assert!(0x1200 + names.len() < 0x1800);
        put(&mut image, 0x1200, &names);
        for (index, (tag, value)) in [
            (5_i64, 0x1200_u64),
            (10, names.len() as u64),
            (6, 0x1800),
            (39, (self.imports.len() as u64 + 1) * 24),
            (11, 24),
            (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
            (0x6100_0015, library_name | (1 << 32) | (3 << 48)),
            (7, 0x2000),
            (8, self.imports.len() as u64 * 24),
            (9, 24),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            put(&mut image, 0x1000 + index * 16, &tag.to_le_bytes());
            put(&mut image, 0x1008 + index * 16, &value.to_le_bytes());
        }
        create_input(image)
    }
}

#[test]
fn native_json_scalar_lifecycle_preserves_fields_and_returns_actual_reference_addresses() {
    let mut p = Program::new();
    p.invoke(0, 0);
    p.pointer(0);
    p.store(0, 42);
    p.store(8, 17);
    p.store(24, 0x5a5a5a5a);
    p.argument(0xface_cafe_0000_0001);
    p.invoke(4, 0);
    p.word(0, 0);
    p.word(8, 0);
    p.dword(24, 0x5a5a5a5a);
    p.invoke(8, 0);
    p.result(1);
    p.invoke(9, 0);
    p.pointer(16);
    p.code.extend([0x0f, 0xb6, 0]);
    p.result(1);
    p.invoke(10, 0);
    p.code.extend([0x48, 0x8b, 0]);
    p.result(0);
    p.argument(i64::MIN as u64);
    p.invoke(5, 0);
    p.invoke(10, 0);
    p.pointer(16);
    p.code.extend([0x48, 0x8b, 0]);
    p.result(i64::MIN as u64);
    p.argument(u64::MAX);
    p.invoke(6, 0);
    p.invoke(8, 0);
    p.result(3);
    p.source(0);
    p.invoke(12, 64);
    p.pointer(64);
    p.invoke(10, 64);
    p.pointer(80);
    p.code.extend([0x48, 0x8b, 0]);
    p.result(u64::MAX);
    p.dword(88, 0);
    p.source(64);
    p.invoke(13, 0);
    p.pointer(0);
    p.word(16, u64::MAX);
    p.dword(24, 0x5a5a5a5a);
    p.store(64, 42);
    p.store(72, 17);
    p.store(88, 0x0000_0003_9292_9292);
    p.invoke(15, 64);
    p.word(64, 42);
    p.word(72, 17);
    p.word(80, 0);
    p.dword(88, 0x92929292);
    p.invoke(8, 64);
    p.result(0);
    p.invoke(14, 64);
    p.word(64, 0);
    p.word(72, 0);
    p.dword(88, 0x92929292);
    p.invoke(16, 64);
    p.result(0);
    p.argument(4);
    p.invoke(7, 0);
    p.invoke(11, 0);
    p.pointer(16);
    p.code.extend([0x48, 0x8b, 0]);
    p.result(0);
    p.argument(1);
    p.invoke(1, 64);
    p.pointer(64);
    p.dword(88, 0);
    p.argument(0x1234_5678_90ab_cdef);
    p.invoke(2, 0);
    p.pointer(0);
    p.word(16, 0x1234_5678_90ab_cdef);
    p.argument(2);
    p.invoke(3, 0);
    p.pointer(0);
    p.word(16, 0);
    p.invoke(17, 112);
    p.code
        .extend([0x48, 0x8b, 0x44, 0x24, 112, 0x48, 0x89, 0x44, 0x24, 120]);
    p.invoke(18, 112);
    p.code
        .extend([0x48, 0x8b, 0x44, 0x24, 112, 0x48, 0x3b, 0x44, 0x24, 120]);
    p.checked();
    let image = p.finish(b"Json2", b"Json2");
    let (status, fields, text) = probe(&image);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_observed_json_scope_sets_and_reads_boolean_references() {
    let mut p = Program::new();
    p.invoke(0, 0);
    p.argument(1);
    p.invoke(4, 0);
    p.invoke(8, 0);
    p.result(1);
    p.invoke(9, 0);
    p.pointer(16);
    p.code.extend([0x0f, 0xb6, 0]);
    p.result(1);
    p.invoke(14, 0);
    p.invoke(8, 0);
    p.result(0);
    let image = p.finish(b"libSceJson", b"libSceJson2");
    let (status, fields, text) = probe(&image);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_json_setter_cannot_discard_unknown_owned_string_storage() {
    let mut p = Program::new();
    p.invoke(0, 0);
    p.store(16, u64::MAX);
    p.store(24, 5_u64 << 32);
    p.argument(1);
    p.invoke(4, 0);
    let image = p.finish(b"libSceJson", b"libSceJson2");
    let (status, fields, text) = probe(&image);
    assert_ne!(status, 0, "{text}");
    assert_eq!(fields["code"], "invalid-json-string", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
}

#[test]
fn native_json_double_arguments_preserve_raw_bits_across_capture_and_resume() {
    for bits in [
        0,
        0x8000_0000_0000_0000,
        0x3ff8_0000_0000_0000,
        0x7ff0_0000_0000_0000,
        0xfff0_0000_0000_0000,
        0x7ff0_0000_0000_0042,
        0xfff8_1234_5678_9abc,
    ] {
        let mut p = Program::new();
        for offset in [0, 8, 16, 24] {
            p.store(offset, u64::MAX);
        }
        p.real_argument(bits);
        p.invoke(19, 0);
        p.pointer(0);
        p.preserved_real_argument(bits);
        p.word(0, 0);
        p.word(8, 0);
        p.word(16, bits);
        p.word(24, 4_u64 << 32);
        p.invoke(11, 0);
        p.pointer(16);
        p.code.extend([0x48, 0x8b, 0]);
        p.result(bits);

        // Setter clears an existing scalar while retaining its padding.
        p.store(0, 0x1234);
        p.store(8, 0x5678);
        p.store(24, (3_u64 << 32) | 0x5a5a_5a5a);
        p.real_argument(bits);
        p.invoke(20, 0);
        p.result(0);
        p.preserved_real_argument(bits);
        p.word(0, 0);
        p.word(8, 0);
        p.word(16, bits);
        p.word(24, (4_u64 << 32) | 0x5a5a_5a5a);
        p.invoke(11, 0);
        p.pointer(16);
        p.code.extend([0x48, 0x8b, 0]);
        p.result(bits);
        for function in [19, 20] {
            p.real_argument(bits);
            p.code.extend([0x31, 0xff]);
            p.call(function);
            p.result(0);
        }
        let image = p.finish(b"libSceJson", b"libSceJson2");
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
    }
}

#[test]
fn native_json_double_setter_retains_unknown_owned_storage_refusal() {
    let mut p = Program::new();
    p.invoke(0, 0);
    p.store(16, u64::MAX);
    p.store(24, 5_u64 << 32);
    p.real_argument(0x8000_0000_0000_0000);
    p.invoke(20, 0);
    let image = p.finish(b"libSceJson", b"libSceJson2");
    let (status, fields, text) = probe(&image);
    assert_ne!(status, 0, "{text}");
    assert_eq!(fields["code"], "invalid-json-string", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
}
