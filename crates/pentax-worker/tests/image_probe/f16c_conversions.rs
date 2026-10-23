// SPDX-License-Identifier: GPL-2.0-only
//! Original F16C guest consumers of binary16 bits, rounding, widths and state.
//! Fixed integer literals are independent expected results; no host F16C is required.
use super::*;

const ENTRY: i32 = 0x1000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const MXCSR: u32 = 0x1f84;
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
        self.word(
            1032,
            u64::from(MXCSR | (((self.base as u32 >> 12) & 3) << 13)),
        );
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

// DAZ is optional in the existing host-compatible MXCSR profile, independently
// of the integer-only F16C mediator. Read only its architectural mask; never
// retain, format or assert the saved host register payload.
#[allow(unsafe_code)]
mod host_control {
    pub(super) fn daz_bit() -> u32 {
        static MASK: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *MASK.get_or_init(|| {
            if !std::is_x86_feature_detected!("fxsr") {
                return 0;
            }
            #[repr(C, align(16))]
            struct Snapshot([u8; 512]);
            let mut snapshot = Snapshot([0; 512]);
            // SAFETY: FXSR is checked; the exclusive local512-byte destination
            // has required16-byte alignment. FXSAVE changes no host state.
            unsafe { std::arch::x86_64::_fxsave64(snapshot.0.as_mut_ptr()) };
            let mask = u32::from_le_bytes(snapshot.0[28..32].try_into().unwrap());
            (if mask == 0 { 0xffbf } else { mask }) & 0x40
        })
    }
}
const CANARY_VECTOR: [u64; 4] = [CANARY; 4];
const SOURCE: u8 = 14;
const DESTINATION: u8 = 15;
const INPUT: u32 = 64;
const OUTPUT: u32 = 128;
const CONTROL: u32 = 176;
#[derive(Clone, Copy)]
enum Location {
    Register,
    Alias,
    Memory,
}
#[derive(Clone, Copy)]
struct Form {
    narrow: bool,
    wide: bool,
    location: Location,
}
impl Form {
    fn width(self) -> usize {
        if self.wide { 16 } else { 8 }
    }
    fn source(self) -> u8 {
        if matches!(self.location, Location::Alias) {
            DESTINATION
        } else {
            SOURCE
        }
    }
}
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
    fn control(&mut self, value: u32) {
        self.word(CONTROL, u64::from(value));
        self.bytes.extend([0x0f, 0xae, 0x94, 0x24]);
        self.bytes.extend(CONTROL.to_le_bytes());
    }
    fn check_control(&mut self, value: u32) {
        self.bytes.extend([0x0f, 0xae, 0x9c, 0x24]);
        self.bytes.extend(CONTROL.to_le_bytes());
        self.check(CONTROL, u64::from(value));
    }
    fn flags(&mut self) {
        self.bytes.extend([0x48, 0x39, 0xc0, 0xf9]);
    }
    fn check_flags(&mut self) {
        for condition in [0x83, 0x85, 0x8b, 0x88, 0x80] {
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
        self.check_x87();
    }
}
fn single_words(lanes: [u32; 8]) -> [u64; 4] {
    std::array::from_fn(|i| u64::from(lanes[i * 2]) | (u64::from(lanes[i * 2 + 1]) << 32))
}
fn half_words(lanes: [u16; 8]) -> [u64; 4] {
    [
        lanes[..4]
            .iter()
            .enumerate()
            .fold(0, |v, (i, x)| v | (u64::from(*x) << (i * 16))),
        lanes[4..]
            .iter()
            .enumerate()
            .fold(0, |v, (i, x)| v | (u64::from(*x) << (i * 16))),
        0,
        0,
    ]
}
fn instruction(p: &mut Code, form: Form, immediate: u8, boundary: bool) {
    let memory = matches!(form.location, Location::Memory);
    let source = form.source();
    // Reserved vvvv remains1111; PS2PH ModRM.reg is the source, PH2PS is destination.
    p.bytes.extend([
        0xc4,
        if memory {
            if form.narrow { 0x63 } else { 0x62 }
        } else if form.narrow {
            0x43
        } else {
            0x42
        },
        0x79 | (u8::from(form.wide) << 2),
        if form.narrow { 0x1d } else { 0x13 },
    ]);
    if memory {
        let register = if form.narrow { source } else { DESTINATION };
        if boundary {
            // RAX is a scratch pointer to an exact admitted end-of-segment span.
            p.bytes.push((register & 7) << 3);
        } else {
            p.bytes.extend([0x84 | ((register & 7) << 3), 0x24]);
            p.bytes
                .extend(if form.narrow { OUTPUT } else { INPUT }.to_le_bytes());
        }
    } else {
        p.bytes.push(
            0xc0 | if form.narrow {
                ((source & 7) << 3) | (DESTINATION & 7)
            } else {
                ((DESTINATION & 7) << 3) | (source & 7)
            },
        );
    }
    if form.narrow {
        p.bytes.push(immediate);
    }
}
fn image(code: &[u8], tail: &[u8]) -> Input {
    let mut bytes = module_bytes(&[], false, None);
    bytes.resize(0xc000, 0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 56, &2_u16.to_le_bytes());
    for offset in [96, 104] {
        put(&mut bytes, offset, &0x9000_u64.to_le_bytes());
    }
    put(&mut bytes, 112, &0x1000_u64.to_le_bytes());
    put(&mut bytes, 120, &1_u32.to_le_bytes());
    put(&mut bytes, 124, &6_u32.to_le_bytes());
    for (offset, value) in [
        (128, 0xa000_u64),
        (136, 0xa000),
        (152, 0x2000),
        (160, 0x2000),
        (168, 0x1000),
    ] {
        put(&mut bytes, offset, &value.to_le_bytes());
    }
    assert!(ENTRY as usize + code.len() < 0x9000);
    assert!(tail.len() <= 32);
    put(&mut bytes, ENTRY as usize, code);
    put(&mut bytes, 0xc000 - tail.len(), tail);
    create_input(bytes)
}
fn start() -> Code {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    p
}
fn returned(source: &Input, status_flags: u32) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["fp_status"], status_flags.to_string(), "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert!(fields["vector_ops"].parse::<u64>().unwrap() > 0, "{text}");
}
fn narrowing(form: Form) -> Input {
    let input = single_words([
        0x3f801000, 0xbf801000, 0x3f803000, 0xbf803000, 0x38800000, 0x33800000, 0x477fe000,
        0x7f800000,
    ]);
    let mut p = start();
    let mut last = [0; 4];
    for rc in 0..4 {
        for immediate in [0, 1, 2, 3, 4, 0x88, 0x8f] {
            let rounding = if immediate & 4 == 0 {
                immediate & 3
            } else {
                rc
            };
            let low = match rounding {
                0 => [0x3c00, 0xbc00, 0x3c02, 0xbc02],
                1 => [0x3c00, 0xbc01, 0x3c01, 0xbc02],
                2 => [0x3c01, 0xbc00, 0x3c02, 0xbc01],
                _ => [0x3c00, 0xbc00, 0x3c01, 0xbc01],
            };
            last = half_words([
                low[0],
                low[1],
                low[2],
                low[3],
                if form.wide { 0x0400 } else { 0 },
                if form.wide { 1 } else { 0 },
                if form.wide { 0x7bff } else { 0 },
                if form.wide { 0x7c00 } else { 0 },
            ]);
            p.fill(INPUT, input);
            p.vector_at(true, form.source(), INPUT);
            p.fill(OUTPUT, CANARY_VECTOR);
            p.word(OUTPUT - 8, CANARY);
            p.word(OUTPUT + 32, CANARY);
            let control = (0x9f84 | host_control::daz_bit()) | (u32::from(rc) << 13); // Optional DAZ and FTZ cannot alter these normal inputs.
            p.control(control);
            p.flags();
            instruction(&mut p, form, immediate, false);
            p.check_flags();
            if matches!(form.location, Location::Memory) {
                p.check_words(
                    OUTPUT,
                    [
                        last[0],
                        if form.wide { last[1] } else { CANARY },
                        CANARY,
                        CANARY,
                    ],
                );
                p.check(OUTPUT - 8, CANARY);
                p.check(OUTPUT + 32, CANARY);
            } else {
                p.vector_at(false, DESTINATION, OUTPUT);
                p.check_words(OUTPUT, last);
            }
            p.check_control(control | 0x20); // Immediate bit3 does not suppress precision.
        }
    }
    match form.location {
        Location::Alias => p.check_retained(&[(DESTINATION, last)]),
        Location::Register => p.check_retained(&[(SOURCE, input), (DESTINATION, last)]),
        Location::Memory => p.check_retained(&[(SOURCE, input)]),
    }
    image(&p.finish(42), &[])
}
fn widening(wide: bool) -> Input {
    let input = half_words([0, 0x8000, 0x3c00, 0xc000, 1, 0x03ff, 0x0400, 0x7c00]);
    let output = single_words([
        0,
        0x80000000,
        0x3f800000,
        0xc0000000,
        if wide { 0x33800000 } else { 0 },
        if wide { 0x387fc000 } else { 0 },
        if wide { 0x38800000 } else { 0 },
        if wide { 0x7f800000 } else { 0 },
    ]);
    let mut p = start();
    for location in [Location::Register, Location::Alias, Location::Memory] {
        for rc in 0..4 {
            let form = Form {
                narrow: false,
                wide,
                location,
            };
            p.fill(INPUT, input);
            p.vector_at(true, form.source(), INPUT);
            let control = (0x9f84 | host_control::daz_bit()) | (rc << 13);
            p.control(control);
            p.flags();
            instruction(&mut p, form, 0, false);
            p.check_flags();
            p.vector_at(false, DESTINATION, OUTPUT);
            p.check_words(OUTPUT, output);
            p.check_control(control); // Even half subnormals convert exactly without DE or UE.
        }
    }
    p.check_retained(&[(SOURCE, input), (DESTINATION, output)]);
    image(&p.finish(42), &[])
}

fn boundary(form: Form) -> Input {
    let input = single_words([
        0, 0x80000000, 0x3f800000, 0xc0000000, 0x38800000, 0x33800000, 0x477fe000, 0x7f800000,
    ]);
    let halves = half_words([0, 0x8000, 0x3c00, 0xc000, 1, 0x03ff, 0x0400, 0x7c00]);
    let mut p = start();
    p.control(0x9f84 | host_control::daz_bit());
    let (expected, tail) = if form.narrow {
        p.fill(INPUT, input);
        p.vector_at(true, SOURCE, INPUT);
        (
            half_words([
                0,
                0x8000,
                0x3c00,
                0xc000,
                if form.wide { 0x0400 } else { 0 },
                if form.wide { 1 } else { 0 },
                if form.wide { 0x7bff } else { 0 },
                if form.wide { 0x7c00 } else { 0 },
            ]),
            CANARY_VECTOR
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
    } else {
        (
            single_words([
                0,
                0x80000000,
                0x3f800000,
                0xc0000000,
                if form.wide { 0x33800000 } else { 0 },
                if form.wide { 0x387fc000 } else { 0 },
                if form.wide { 0x38800000 } else { 0 },
                if form.wide { 0x7f800000 } else { 0 },
            ]),
            halves[..form.width() / 8]
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<_>>(),
        )
    };
    p.flags();
    p.path(0, 0xc000 - form.width() as i32);
    instruction(&mut p, form, 0, true);
    p.check_flags();
    if form.narrow {
        for (index, value) in expected[..form.width() / 8].iter().enumerate() {
            p.path(0, 0xc000 - form.width() as i32 + index as i32 * 8);
            p.bytes.extend([0x48, 0x8b, 0]);
            p.expect(0, *value);
        }
        p.path(0, 0xc000 - form.width() as i32 - 8);
        p.bytes.extend([0x48, 0x8b, 0]);
        p.expect(0, CANARY);
        p.check_retained(&[(SOURCE, input)]);
    } else {
        p.vector_at(false, DESTINATION, OUTPUT);
        p.check_words(OUTPUT, expected);
        p.check_retained(&[(DESTINATION, expected)]);
    }
    p.check_control(0x9f84 | host_control::daz_bit());
    image(&p.finish(42), &tail)
}
fn fault(form: Form, address: i32, control: u32) -> Input {
    let mut p = start();
    p.fill(INPUT, single_words([0x3f801000; 8]));
    p.vector_at(true, SOURCE, INPUT);
    p.control(control);
    p.path(0, address);
    instruction(&mut p, form, 0, true);
    image(&p.finish(42), &[])
}
fn rejected(source: &Input, code: &str, failure: Option<&str>, planned: Option<&str>) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], code, "{text}");
    if let Some(failure) = failure {
        assert_eq!(fields["runtime_failure"], failure, "{text}");
    }
    if let Some(planned) = planned {
        assert_eq!(fields["planned_exit"], planned, "{text}");
    }
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
}

#[derive(Clone, Copy)]
enum Pattern {
    Nan,
    Overflow,
    Tiny,
    InputDenormal,
    TinyCarry,
    NormalCarry,
}
fn exceptional(wide: bool, pattern: Pattern) -> Input {
    let low = match pattern {
        Pattern::Nan => [0x7fc02000, 0xff802000, 0x7f800000, 0xff800000],
        Pattern::Overflow => [0x47800000, 0xc7800000, 0x7f7fffff, 0xff7fffff],
        Pattern::Tiny => [0x33000000, 0xb3000000, 0x33000001, 0xb3000001],
        Pattern::InputDenormal => [1, 0x80000001, 0x007fffff, 0x807fffff],
        Pattern::TinyCarry => [0x387fe800, 0xb87fe800, 0x387fe800, 0xb87fe800],
        Pattern::NormalCarry => [0x387ff800, 0xb87ff800, 0x387ff800, 0xb87ff800],
    };
    let input = single_words([
        low[0], low[1], low[2], low[3], 0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000,
    ]);
    let form = Form {
        narrow: true,
        wide,
        location: Location::Register,
    };
    let mut p = start();
    let mut last = [0; 4];
    for rc in 0..4 {
        for daz in [false, host_control::daz_bit() != 0] {
            let (low, status): ([u16; 4], u32) = match pattern {
                Pattern::Nan => ([0x7e01, 0xfe01, 0x7c00, 0xfc00], 1),
                Pattern::Overflow => (
                    match rc {
                        0 => [0x7c00, 0xfc00, 0x7c00, 0xfc00],
                        1 => [0x7bff, 0xfc00, 0x7bff, 0xfc00],
                        2 => [0x7c00, 0xfbff, 0x7c00, 0xfbff],
                        _ => [0x7bff, 0xfbff, 0x7bff, 0xfbff],
                    },
                    0x28,
                ),
                Pattern::Tiny => (
                    match rc {
                        0 => [0, 0x8000, 1, 0x8001],
                        1 => [0, 0x8001, 0, 0x8001],
                        2 => [1, 0x8000, 1, 0x8000],
                        _ => [0, 0x8000, 0, 0x8000],
                    },
                    0x30,
                ),
                Pattern::InputDenormal if daz => ([0, 0x8000, 0, 0x8000], 0),
                Pattern::InputDenormal => (
                    match rc {
                        1 => [0, 0x8001, 0, 0x8001],
                        2 => [1, 0x8000, 1, 0x8000],
                        _ => [0, 0x8000, 0, 0x8000],
                    },
                    0x32,
                ),
                Pattern::TinyCarry => ([0x0400, 0x8400, 0x0400, 0x8400], 0x30),
                Pattern::NormalCarry => ([0x0400, 0x8400, 0x0400, 0x8400], 0x20),
            };
            last = half_words([
                low[0],
                low[1],
                low[2],
                low[3],
                if wide { 0x3c00 } else { 0 },
                if wide { 0x3c00 } else { 0 },
                if wide { 0x3c00 } else { 0 },
                if wide { 0x3c00 } else { 0 },
            ]);
            p.fill(INPUT, input);
            p.vector_at(true, SOURCE, INPUT);
            let control = 0x9f84 | (rc << 13) | if daz { 0x40 } else { 0 };
            p.control(control);
            p.flags();
            // The two rounded-minnormal probes deliberately force RN despite all MXCSR RCs.
            let immediate = if matches!(pattern, Pattern::TinyCarry | Pattern::NormalCarry) {
                0x88
            } else {
                4
            };
            instruction(&mut p, form, immediate, false);
            p.check_flags();
            p.vector_at(false, DESTINATION, OUTPUT);
            p.check_words(OUTPUT, last);
            p.check_control(control | status);
        }
    }
    p.check_retained(&[(SOURCE, input), (DESTINATION, last)]);
    image(&p.finish(42), &[])
}
fn widening_nan(wide: bool) -> Input {
    let input = half_words([0x7e01, 0xfc01, 0x7c00, 0xfc00, 1, 0x8001, 0x03ff, 0x83ff]);
    let expected = single_words([
        0x7fc02000,
        0xffc02000,
        0x7f800000,
        0xff800000,
        if wide { 0x33800000 } else { 0 },
        if wide { 0xb3800000 } else { 0 },
        if wide { 0x387fc000 } else { 0 },
        if wide { 0xb87fc000 } else { 0 },
    ]);
    let mut p = start();
    for location in [Location::Register, Location::Alias, Location::Memory] {
        let form = Form {
            narrow: false,
            wide,
            location,
        };
        p.fill(INPUT, input);
        p.vector_at(true, form.source(), INPUT);
        p.control(0x9f84 | host_control::daz_bit());
        p.flags();
        instruction(&mut p, form, 0, false);
        p.check_flags();
        p.vector_at(false, DESTINATION, OUTPUT);
        p.check_words(OUTPUT, expected);
        p.check_control(0x9f85 | host_control::daz_bit());
    }
    p.check_retained(&[(SOURCE, input), (DESTINATION, expected)]);
    image(&p.finish(42), &[])
}
#[test]
fn native_f16c_conversions_consume_literal_bits_rounding_aliases_status_and_exact_memory_widths() {
    let mut count = 0;
    for wide in [false, true] {
        for location in [Location::Register, Location::Alias, Location::Memory] {
            returned(
                &narrowing(Form {
                    narrow: true,
                    wide,
                    location,
                }),
                0x24,
            );
            count += 1;
        }
        returned(&widening(wide), 4);
        count += 1;
        returned(&widening_nan(wide), 5);
        count += 1;
        for pattern in [
            Pattern::Nan,
            Pattern::Overflow,
            Pattern::Tiny,
            Pattern::InputDenormal,
            Pattern::TinyCarry,
            Pattern::NormalCarry,
        ] {
            let flags = match pattern {
                Pattern::Nan => 5,
                Pattern::Overflow => 0x2c,
                Pattern::Tiny | Pattern::TinyCarry => 0x34,
                Pattern::InputDenormal => {
                    if host_control::daz_bit() != 0 {
                        4
                    } else {
                        0x36
                    }
                }
                Pattern::NormalCarry => 0x24,
            };
            returned(&exceptional(wide, pattern), flags);
            count += 1;
        }
        for narrow in [true, false] {
            returned(
                &boundary(Form {
                    narrow,
                    wide,
                    location: Location::Memory,
                }),
                4,
            );
            count += 1;
        }
    }
    assert_eq!(count, 26);
    println!("f16c_native_daz_available={}", host_control::daz_bit() != 0);
    println!("f16c_native_available=true f16c_native_programs={count} f16c_native_returns={count}");
}
#[test]
fn native_f16c_conversions_refuse_unmasked_state_bad_memory_and_unadmitted_encodings() {
    let mut count = 0;
    for wide in [false, true] {
        for narrow in [true, false] {
            let form = Form {
                narrow,
                wide,
                location: Location::Memory,
            };
            // Full8/16B access crosses the actual RW segment end, never a speculative wide load.
            rejected(
                &fault(form, 0xc000 - form.width() as i32 + 1, 0x1f80),
                "native-dispatch",
                Some("memory-unmapped"),
                Some(if narrow { "Vcvtps2ph" } else { "Vcvtph2ps" }),
            );
            count += 1;
            // LDMXCSR itself refuses the unmasked installation before the F16C or memory access.
            // Platform direct-state fixtures separately exercise F16C InvalidState.
            rejected(
                &fault(form, 0xc000, 0x0f80),
                "native-dispatch",
                Some("invalid-cpu-state"),
                Some("Ldmxcsr"),
            );
            count += 1;
        }
        let form = Form {
            narrow: true,
            wide,
            location: Location::Memory,
        };
        // Executable source is read-only; neither code bytes nor CPU outputs may publish.
        rejected(
            &fault(form, ENTRY, 0x1f80),
            "native-dispatch",
            Some("memory-permission"),
            Some("Vcvtps2ph"),
        );
        count += 1;
    }
    for narrow in [true, false] {
        let mut p = start();
        p.control(0x1f80);
        let form = Form {
            narrow,
            wide: false,
            location: Location::Register,
        };
        p.bytes.push(0xf3);
        instruction(&mut p, form, 0, false);
        // Iced rejects a preceding mandatory F3 before VEX at translation.
        // This malformed encoding never reaches the checked F16C mediator.
        rejected(&image(&p.finish(42), &[]), "block-translation", None, None);
        count += 1;
    }
    // EVEX widens the architectural family beyond the four exact VEX forms.
    for encoding in [
        &[0x62, 0x03, 0x7d, 0x48, 0x1d, 0xf7, 0][..],
        &[0x62, 0x02, 0x7d, 0x48, 0x13, 0xfe][..],
    ] {
        let mut p = start();
        p.bytes.extend(encoding);
        rejected(
            &image(&p.finish(42), &[]),
            "unsupported-instruction",
            None,
            None,
        );
        count += 1;
    }
    assert_eq!(count, 14);
    println!("f16c_native_refusal_programs={count} f16c_native_refusals={count}");
}
