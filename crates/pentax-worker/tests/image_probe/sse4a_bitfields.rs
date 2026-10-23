// SPDX-License-Identifier: GPL-2.0-only
//! Original SSE4a bitfield guest consumers with fixed integer result literals.
//! The checked software profile needs no host SSE4a. Upper XMM preservation is
//! the emulator's deterministic policy for architecturally undefined bits;
//! legacy upper YMM preservation is architectural. No host oracle is used.
use super::*;

const ENTRY: i32 = 0x1000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const MXCSR: u32 = 0x5f84;
const HELD: u64 = 0x8000_0000_0000_0007;
const GPRS: [u8; 12] = [1, 2, 3, 6, 7, 8, 9, 11, 12, 13, 14, 15];

struct Code {
    base: i32,
    bytes: Vec<u8>,
    failed: Vec<usize>,
}
impl Code {
    fn new(base: i32) -> Self {
        Self {
            base,
            bytes: vec![0x55, 0x48, 0x81, 0xec, 0, 8, 0, 0],
            failed: Vec::new(),
        }
    }
    fn reg(&mut self, register: u8, value: u64) {
        self.bytes
            .extend([0x48 | u8::from(register >= 8), 0xb8 + (register & 7)]);
        self.bytes.extend(value.to_le_bytes());
    }
    fn stack(&mut self, opcode: u8, register: u8, offset: u32) {
        self.bytes.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            opcode,
            0x84 | ((register & 7) << 3),
            0x24,
        ]);
        self.bytes.extend(offset.to_le_bytes());
    }
    fn word(&mut self, offset: u32, value: u64) {
        self.reg(0, value);
        self.stack(0x89, 0, offset);
    }
    fn bad(&mut self) {
        self.bytes.extend([0x0f, 0x85, 0, 0, 0, 0]);
        self.failed.push(self.bytes.len() - 4);
    }
    fn expect(&mut self, register: u8, value: u64) {
        self.reg(10, value);
        self.bytes
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        self.bad();
    }
    fn check(&mut self, offset: u32, value: u64) {
        self.stack(0x8b, 0, offset);
        self.expect(0, value);
    }
    fn path(&mut self, register: u8, target: i32) {
        rip(
            &mut self.bytes,
            self.base,
            &[
                0x48 | if register >= 8 { 4 } else { 0 },
                0x8d,
                ((register & 7) << 3) | 5,
            ],
            target,
        );
    }
    fn vector(&mut self, load: bool, register: u8) {
        self.bytes.extend([
            0xc5,
            if register >= 8 { 0x7c } else { 0xfc },
            if load { 0x10 } else { 0x11 },
            0x84 | ((register & 7) << 3),
            0x24,
        ]);
        self.bytes.extend(1136_u32.to_le_bytes());
    }
    fn seed_context(&mut self) {
        self.stack(0x89, 4, 1024);
        self.stack(0x89, 5, 1176);
        self.word(1032, u64::from(MXCSR));
        self.bytes.extend([0x0f, 0xae, 0x94, 0x24]);
        self.bytes.extend(1032_u32.to_le_bytes());
        for register in 0..16 {
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.word(1136 + i as u32 * 8, value ^ ((self.base as u64) << 32));
            }
            self.vector(true, register);
        }
        for register in GPRS {
            self.reg(register, gpr_pattern(register) ^ self.base as u64);
        }
    }
    fn hold_x87(&mut self) {
        self.bytes.extend([0xdb, 0xe3]);
        self.word(1192, HELD);
        self.word(1200, 0x3fff);
        self.bytes.extend([0xdb, 0xac, 0x24]);
        self.bytes.extend(1192_u32.to_le_bytes());
    }
    fn check_x87(&mut self) {
        self.word(1216, CANARY);
        self.word(1224, CANARY);
        self.bytes.extend([0xdb, 0xbc, 0x24]);
        self.bytes.extend(1216_u32.to_le_bytes());
        self.check(1216, HELD);
        self.check(1224, (CANARY & !0xffff) | 0x3fff);
        self.word(1232, CANARY);
        self.bytes.extend([0xdd, 0xbc, 0x24]);
        self.bytes.extend(1232_u32.to_le_bytes());
        self.check(1232, CANARY & !0xffff);
    }
    fn finish(mut self, value: u64) -> Vec<u8> {
        self.reg(0, value);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 8, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x8000);
        self.bytes
    }
}
fn gpr_pattern(register: u8) -> u64 {
    0x3141_5926_5358_0000 | u64::from(register)
}
fn vector_pattern(register: u8) -> [u64; 4] {
    let tag = u64::from(register) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}

const INPUT: u32 = 64;
const SOURCE_INPUT: u32 = 96;
const OUTPUT: u32 = 128;
const SOURCE: u8 = 15;
const DESTINATION: u8 = 14;
const DESTINATION_WORDS: [u64; 4] = [
    0x0123_4567_89ab_cdef,
    0x7ff0_0000_0000_0055,
    0x8877_6655_4433_2211,
    0x89ab_cdef_0123_4567,
];
const SOURCE_WORDS: [u64; 4] = [
    0xfedc_ba98_7654_3210,
    0xfff8_0000_0000_00aa,
    0x1122_3344_5566_7788,
    0x7766_5544_3322_1100,
];

impl Code {
    fn vector_at(&mut self, load: bool, register: u8, offset: u32) {
        self.bytes.extend([
            0xc5,
            if register >= 8 { 0x7c } else { 0xfc },
            if load { 0x10 } else { 0x11 },
            0x84 | ((register & 7) << 3),
            0x24,
        ]);
        self.bytes.extend(offset.to_le_bytes());
    }
    fn flags(&mut self, overflow: bool) {
        self.reg(10, gpr_pattern(10));
        if overflow {
            self.reg(0, 0x7fff_ffff_ffff_ffff);
            self.bytes.extend([0x48, 0x83, 0xc0, 1]);
        } else {
            self.reg(0, CANARY);
            self.bytes.extend([0x48, 0x39, 0xc0, 0xf9]);
        }
    }
    fn check_flags(&mut self, overflow: bool) {
        let failures = if overflow {
            [0x82, 0x84, 0x8b, 0x89, 0x81]
        } else {
            [0x83, 0x85, 0x8b, 0x88, 0x80]
        };
        for condition in failures {
            self.bytes.extend([0x0f, condition, 0, 0, 0, 0]);
            self.failed.push(self.bytes.len() - 4);
        }
    }
    fn fill(&mut self, offset: u32, words: [u64; 4]) {
        for (index, value) in words.into_iter().enumerate() {
            self.word(offset + index as u32 * 8, value);
        }
    }
    fn check_words(&mut self, offset: u32, words: [u64; 4]) {
        for (index, value) in words.into_iter().enumerate() {
            self.check(offset + index as u32 * 8, value);
        }
    }
    fn check_retained(&mut self, changed: &[(u8, [u64; 4])]) {
        for register in GPRS {
            self.expect(register, gpr_pattern(register) ^ ENTRY as u64);
        }
        for register in 0..16 {
            self.vector(false, register);
            let expected = changed
                .iter()
                .find(|(r, _)| *r == register)
                .map(|(_, v)| *v)
                .unwrap_or_else(|| vector_pattern(register).map(|v| v ^ ((ENTRY as u64) << 32)));
            self.check_words(1136, expected);
        }
        self.stack(0x8b, 0, 1024);
        self.bytes.extend([0x48, 0x39, 0xc4]);
        self.bad();
        self.stack(0x8b, 0, 1176);
        self.bytes.extend([0x48, 0x39, 0xc5]);
        self.bad();
        self.word(1112, CANARY);
        self.bytes.extend([0x0f, 0xae, 0x9c, 0x24]);
        self.bytes.extend(1112_u32.to_le_bytes());
        self.check(1112, (CANARY & !0xffff_ffff) | u64::from(MXCSR));
        self.check_x87();
    }
}
fn image(main: &[u8]) -> Input {
    let roles = [
        pentax_kernel::Service::Getpid,
        pentax_kernel::Service::ErrnoLocation,
    ];
    let nids: Vec<_> = roles
        .into_iter()
        .map(|role| {
            pentax_kernel::Service::registrations()
                .iter()
                .find(|entry| entry.service == role)
                .unwrap()
                .nid
        })
        .collect();
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x9000, 0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x9000_u64.to_le_bytes());
    put(&mut bytes, 104, &0xc000_u64.to_le_bytes());
    bytes[0x600..0x800].fill(0);
    assert!(main.len() < 0x8000);
    put(&mut bytes, ENTRY as usize, main);
    let mut names = b"\0libkernel\0libkernel\0".to_vec();
    for (index, nid) in nids.iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        names.extend_from_slice(format!("{nid}#D#C\0").as_bytes());
        put(
            &mut bytes,
            0x700 + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x708 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    put(&mut bytes, 0x400, &names);
    for (index, (tag, value)) in [
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, 3 * 24),
        (11, 24),
        (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
        (0x6100_0015, 11 | (1 << 32) | (3 << 48)),
        (7, 0x700),
        (8, 2 * 24),
        (9, 24),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}
#[derive(Clone, Copy)]
enum Form {
    ExtractImmediate,
    ExtractRegister,
    InsertImmediate,
    InsertRegister,
}
impl Form {
    fn insert(self) -> bool {
        matches!(self, Self::InsertImmediate | Self::InsertRegister)
    }
    fn immediate(self) -> bool {
        matches!(self, Self::ExtractImmediate | Self::InsertImmediate)
    }
    fn mnemonic(self) -> &'static str {
        if self.insert() { "Insertq" } else { "Extrq" }
    }
}
const FORMS: [Form; 4] = [
    Form::ExtractImmediate,
    Form::ExtractRegister,
    Form::InsertImmediate,
    Form::InsertRegister,
];
fn instruction(p: &mut Code, form: Form, source: u8, length: u8, index: u8) {
    p.bytes.push(if form.insert() { 0xf2 } else { 0x66 });
    let register = if matches!(form, Form::ExtractImmediate) {
        0
    } else {
        DESTINATION
    };
    let rm = if matches!(form, Form::ExtractImmediate) {
        DESTINATION
    } else {
        source
    };
    let rex = 0x40 | (u8::from(register >= 8) << 2) | u8::from(rm >= 8);
    if rex != 0x40 {
        p.bytes.push(rex);
    }
    p.bytes.extend([
        0x0f,
        if form.immediate() { 0x78 } else { 0x79 },
        0xc0 | ((register & 7) << 3) | (rm & 7),
    ]);
    if form.immediate() {
        p.bytes.extend([length, index]);
    }
}
fn operands(form: Form, length: u8, index: u8) -> [u64; 4] {
    let mut words = SOURCE_WORDS;
    let control = u64::from(length) | (u64::from(index) << 8);
    match form {
        Form::ExtractRegister => words[0] = 0xdead_beef_abcd_0000 | control,
        Form::InsertRegister => words[1] = 0xa5a5_5a5a_cc00_0000 | control,
        _ => {}
    }
    words
}
fn start() -> Code {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    p
}
fn consume(
    p: &mut Code,
    form: Form,
    source: u8,
    length: u8,
    index: u8,
    expected: [u64; 4],
    overflow: bool,
) {
    p.flags(overflow);
    instruction(p, form, source, length, index);
    // MOV snapshots consume even the two scratch GPRs before assertions use them.
    p.stack(0x89, 0, 160);
    p.stack(0x89, 10, 168);
    p.check_flags(overflow);
    p.check(
        160,
        if overflow {
            0x8000_0000_0000_0000
        } else {
            CANARY
        },
    );
    p.check(168, gpr_pattern(10));
    p.vector_at(false, DESTINATION, OUTPUT);
    p.check_words(OUTPUT, expected);
}
fn matrix(form: Form) -> Input {
    let mut p = start();
    // Full-width, interior fields, boundary bit, masked controls and a 63-bit
    // field have independent literal oracles. Register controls include noise
    // outside the two six-bit fields; payload bits are never FP operands.
    let cases = [
        (0, 0, 0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210),
        (8, 8, 0xcd, 0x0123_4567_89ab_10ef),
        (16, 16, 0x89ab, 0x0123_4567_3210_cdef),
        (1, 63, 0, 0x0123_4567_89ab_cdef),
        (63, 1, 0x0091_a2b3_c4d5_e6f7, 0xfdb9_7530_eca8_6421),
        (0xc8, 0xc8, 0xcd, 0x0123_4567_89ab_10ef),
    ];
    let mut last_source = SOURCE_WORDS;
    for (iteration, (length, index, extracted, inserted)) in cases.into_iter().enumerate() {
        p.fill(INPUT, DESTINATION_WORDS);
        p.vector_at(true, DESTINATION, INPUT);
        last_source = operands(form, length, index);
        p.fill(SOURCE_INPUT, last_source);
        p.vector_at(true, SOURCE, SOURCE_INPUT);
        let mut expected = DESTINATION_WORDS;
        expected[0] = if form.insert() { inserted } else { extracted };
        consume(
            &mut p,
            form,
            SOURCE,
            length,
            index,
            expected,
            iteration & 1 != 0,
        );
        p.vector_at(false, SOURCE, OUTPUT);
        p.check_words(OUTPUT, last_source);
    }
    // Same-register reads must use the complete pre-publication snapshot.
    let mut alias = DESTINATION_WORDS;
    let result = match form {
        Form::ExtractRegister => {
            alias[0] = 0x0808;
            8
        }
        Form::InsertRegister => {
            alias[1] = 0xa5a5_5a5a_cc00_0808;
            0x0123_4567_89ab_efef
        }
        Form::ExtractImmediate => 0xcd,
        Form::InsertImmediate => 0x0123_4567_89ab_efef,
    };
    p.fill(INPUT, alias);
    p.vector_at(true, DESTINATION, INPUT);
    let mut expected = alias;
    expected[0] = result;
    consume(&mut p, form, DESTINATION, 8, 8, expected, true);
    p.check_retained(&[(DESTINATION, expected), (SOURCE, last_source)]);
    image(&p.finish(42))
}
fn service_continuity() -> Input {
    let mut p = Code::new(ENTRY);
    rip(&mut p.bytes, ENTRY, &[0xff, 0x15], 0xf08); // Actual current-thread errno.
    p.stack(0x89, 0, 1504);
    p.bytes.extend([0xc7, 0, 0xd2, 4, 0, 0]);
    rip(&mut p.bytes, ENTRY, &[0xff, 0x15], 0xf00); // Actual Getpid.
    p.expect(0, 1);
    p.seed_context();
    p.hold_x87();
    p.fill(INPUT, DESTINATION_WORDS);
    p.vector_at(true, DESTINATION, INPUT);
    let source = operands(Form::InsertRegister, 16, 16);
    p.fill(SOURCE_INPUT, source);
    p.vector_at(true, SOURCE, SOURCE_INPUT);
    let mut expected = DESTINATION_WORDS;
    expected[0] = 0x0123_4567_3210_cdef;
    consume(&mut p, Form::InsertRegister, SOURCE, 0, 0, expected, true);
    p.check_retained(&[(DESTINATION, expected), (SOURCE, source)]);
    rip(&mut p.bytes, ENTRY, &[0xff, 0x15], 0xf08);
    p.stack(0x8b, 1, 1504);
    p.bytes.extend([0x48, 0x39, 0xc8]);
    p.bad();
    p.bytes.extend([0x8b, 0]);
    p.expect(0, 1234);
    image(&p.finish(42))
}
fn returned(source: &Input, services: u64) {
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "extended-startup".into(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["service_stops"], services.to_string(), "{text}");
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert!(fields["vector_ops"].parse::<u64>().unwrap() > 0, "{text}");
}
#[test]
fn native_sse4a_bitfields_consume_literal_controls_aliases_legacy_lanes_and_current_thread_context()
{
    for form in FORMS {
        returned(&matrix(form), 0);
    }
    returned(&service_continuity(), 3);
    println!(
        "sse4a_bitfields_native_available=true sse4a_bitfields_native_programs=5 sse4a_bitfields_native_returns=5"
    );
}
#[test]
fn native_sse4a_bitfields_refuse_undefined_control_domains_without_a_fake_result() {
    let mut count = 0;
    for form in FORMS {
        for (length, index) in [(0, 1), (63, 2)] {
            let mut p = start();
            p.fill(INPUT, DESTINATION_WORDS);
            p.vector_at(true, DESTINATION, INPUT);
            p.fill(SOURCE_INPUT, operands(form, length, index));
            p.vector_at(true, SOURCE, SOURCE_INPUT);
            instruction(&mut p, form, SOURCE, length, index);
            let source = image(&p.finish(42));
            let (status, fields, text) = probe(&source);
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "native-dispatch", "{text}");
            assert_eq!(fields["runtime_failure"], "vector-form", "{text}");
            assert_eq!(fields["planned_exit"], form.mnemonic(), "{text}");
            assert_eq!(fields["phase"], "main-entry", "{text}");
            assert_eq!(fields["service_stops"], "0", "{text}");
            assert!(!fields.contains_key("value"), "{text}");
            count += 1;
        }
    }
    assert_eq!(count, 8);
    println!(
        "sse4a_bitfields_native_refusal_programs={count} sse4a_bitfields_native_refusals={count}"
    );
}
