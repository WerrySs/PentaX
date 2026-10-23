// SPDX-License-Identifier: GPL-2.0-only
//! Original marker packet consumers, including post-growth source bytes.
//! Encoding does not submit commands or create any GPU or presentation effect.
use super::*;

const ENTRY: i32 = 0x1000;
const CALLBACK: i32 = 0x6000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const DESCRIPTOR: u32 = 16;
const BUFFER: u32 = 128;
const USERDATA: u32 = 384;
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
            bytes: vec![0x55, 0x48, 0x81, 0xec, 0, 4, 0, 0],
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
    fn ptr(&mut self, register: u8, offset: u32) {
        self.stack(0x8d, register, offset);
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
    fn same_ptr(&mut self, register: u8, offset: u32) {
        self.ptr(10, offset);
        self.bytes
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        self.bad();
    }
    fn pointer_field(&mut self, field: u32, target: u32) {
        self.ptr(0, target);
        self.stack(0x89, 0, field);
    }
    fn field_is_pointer(&mut self, field: u32, target: u32) {
        self.stack(0x8b, 0, field);
        self.same_ptr(0, target);
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
    fn call(&mut self, slot: i32) {
        rip(&mut self.bytes, self.base, &[0xff, 0x15], 0xf00 + slot * 8);
    }
    fn vector(&mut self, load: bool, register: u8) {
        self.bytes.extend([
            0xc5,
            if register >= 8 { 0x7c } else { 0xfc },
            if load { 0x10 } else { 0x11 },
            0x84 | ((register & 7) << 3),
            0x24,
        ]);
        self.bytes.extend(624_u32.to_le_bytes());
    }
    fn seed_context(&mut self) {
        self.stack(0x89, 4, 512);
        self.stack(0x89, 5, 664);
        self.word(
            520,
            u64::from(MXCSR | (((self.base as u32 >> 12) & 3) << 13)),
        );
        self.bytes.extend([0x0f, 0xae, 0x94, 0x24]);
        self.bytes.extend(520_u32.to_le_bytes());
        for register in 0..16 {
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.word(624 + i as u32 * 8, value ^ ((self.base as u64) << 32));
            }
            self.vector(true, register);
        }
        for register in GPRS {
            self.reg(register, gpr_pattern(register) ^ self.base as u64);
        }
    }
    fn check_context(&mut self, environment_base: i32) {
        for register in GPRS {
            // RCX/R11 are real syscall clobbers. The actual call arguments are
            // checked separately; R10 is this original consumer's scratch.
            if matches!(register, 1 | 11 | 7 | 6 | 2) {
                continue;
            }
            self.expect(register, gpr_pattern(register) ^ self.base as u64);
        }
        for register in 0..16 {
            self.vector(false, register);
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.check(624 + i as u32 * 8, value ^ ((self.base as u64) << 32));
            }
        }
        self.stack(0x8b, 0, 512);
        self.bytes.extend([0x48, 0x39, 0xc4]);
        self.bad();
        self.stack(0x8b, 0, 664);
        self.bytes.extend([0x48, 0x39, 0xc5]);
        self.bad();
        self.word(600, CANARY);
        self.bytes.extend([0x0f, 0xae, 0x9c, 0x24]);
        self.bytes.extend(600_u32.to_le_bytes());
        let control = MXCSR | (((environment_base as u32 >> 12) & 3) << 13);
        self.check(600, (CANARY & !0xffff_ffff) | u64::from(control));
    }
    fn hold_x87(&mut self) {
        self.bytes.extend([0xdb, 0xe3]);
        self.word(680, HELD);
        self.word(688, 0x3fff);
        self.bytes.extend([0xdb, 0xac, 0x24]);
        self.bytes.extend(680_u32.to_le_bytes());
    }
    fn check_x87(&mut self) {
        self.word(704, CANARY);
        self.word(712, CANARY);
        self.bytes.extend([0xdb, 0xbc, 0x24]);
        self.bytes.extend(704_u32.to_le_bytes());
        self.check(704, HELD);
        self.check(712, (CANARY & !0xffff) | 0x3fff);
        self.word(720, CANARY);
        self.bytes.extend([0xdd, 0xbc, 0x24]);
        self.bytes.extend(720_u32.to_le_bytes());
        self.check(720, CANARY & !0xffff);
    }
    fn finish(mut self, value: u64) -> Vec<u8> {
        self.reg(0, value);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 4, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x5000);
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

const TEXT: u32 = 448;
#[derive(Clone, Copy, Debug)]
enum Kind {
    AcbPush,
    DcbPush,
    AcbPop,
    DcbPop,
}
impl Kind {
    fn nid(self) -> &'static str {
        match self {
            Self::AcbPush => "cpCILPya5Zk",
            Self::DcbPush => "+kSrjIVxKFE",
            Self::AcbPop => "6mFxkVqdmbQ",
            Self::DcbPop => "H7uZqCoNuWk",
        }
    }
    fn push(self) -> bool {
        matches!(self, Self::AcbPush | Self::DcbPush)
    }
    fn cases(self) -> Vec<Case> {
        if self.push() {
            vec![
                Case::new(None, &[0xc000102c, 0]),
                Case::new(Some(b""), &[0xc000102c, 0]),
                Case::new(Some(b"a"), &[0xc000102c, 0x61]),
                Case::new(Some(b"abc"), &[0xc000102c, 0x00636261]),
                Case::new(Some(b"abcd"), &[0xc001102c, 0x64636261, 0]),
                Case::new(Some(b"abcdefg"), &[0xc001102c, 0x64636261, 0x00676665]),
                Case::new(Some(b"abcdefgh"), &[0xc002102c, 0x64636261, 0x68676665, 0]),
                Case::new(Some(&[0xff, 0x80, b'A']), &[0xc000102c, 0x004180ff]),
                Case::new(
                    Some(b"abcdefghijklmnop"),
                    &[
                        0xc004102c, 0x64636261, 0x68676665, 0x6c6b6a69, 0x706f6e6d, 0,
                    ],
                ),
            ]
        } else {
            vec![Case::new(None, &[0xc0001030, 0])]
        }
    }
    fn growth_case(self) -> Case {
        if self.push() {
            Case::new(Some(b"abcd"), &[0xc001102c, 0x64636261, 0])
        } else {
            Case::new(None, &[0xc0001030, 0])
        }
    }
    fn replacement(self) -> [u8; 8] {
        if matches!(self, Self::AcbPush) {
            *b"0123456\0"
        } else {
            [b'Z', 0, b'p', b'q', b'r', b's', b't', 0]
        }
    }
    fn grown_words(self) -> Vec<u32> {
        match self {
            Self::AcbPush => vec![0xc001102c, 0x33323130, 0x34],
            Self::DcbPush => vec![0xc001102c, 0x7170005a, 0x72],
            Self::AcbPop | Self::DcbPop => vec![0xc0001030, 0],
        }
    }
}
struct Case {
    text: Option<&'static [u8]>,
    words: Vec<u32>,
}
impl Case {
    fn new(text: Option<&'static [u8]>, words: &[u32]) -> Self {
        Self {
            text,
            words: words.to_vec(),
        }
    }
    fn source(&self) -> [u8; 24] {
        let mut bytes = [0xa5; 24];
        if let Some(text) = self.text {
            assert!(text.len() < bytes.len());
            bytes[..text.len()].copy_from_slice(text);
            bytes[text.len()] = 0;
        }
        bytes
    }
}
#[derive(Clone, Copy, Debug)]
enum Scope {
    Canonical,
    Observed,
    WrongPair,
    WrongModuleVersion,
    WrongLibraryVersion,
}
fn image(main: &[u8], callback: &[u8], kind: Kind, scope: Scope) -> Input {
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0xc000, 0);
    bytes[0x400..0x1000].fill(0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0xc000_u64.to_le_bytes());
    put(&mut bytes, 104, &0xc000_u64.to_le_bytes());
    assert!(main.len() < (CALLBACK - ENTRY) as usize);
    assert!(callback.len() < 0x5000);
    put(&mut bytes, ENTRY as usize, main);
    put(&mut bytes, CALLBACK as usize, callback);
    put(&mut bytes, 0xbffe, b"b\0");
    let command_pair = match scope {
        Scope::Canonical => ("Graphics5", "Graphics5"),
        Scope::WrongPair => ("libSceAgc", "Graphics5"),
        _ => ("libSceAgc", "libSceAgc"),
    };
    let mut names = vec![0];
    let mut scopes = Vec::new();
    for (module, library) in [
        command_pair,
        ("Graphics5", "Graphics5"),
        ("libkernel", "libkernel"),
    ] {
        let m = names.len() as u64;
        names.extend_from_slice(module.as_bytes());
        names.push(0);
        let l = names.len() as u64;
        names.extend_from_slice(library.as_bytes());
        names.push(0);
        scopes.push((m, l));
    }
    for (index, nid) in [kind.nid(), "t7PlZ9nt5Lc", "9BcDykPmo1I"]
        .into_iter()
        .enumerate()
    {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        let m = char::from(b'C' + index as u8 * 2);
        let l = char::from(b'D' + index as u8 * 2);
        names.extend_from_slice(format!("{nid}#{l}#{m}\0").as_bytes());
        put(
            &mut bytes,
            0x800 + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x808 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(0x400 + names.len() < 0x600);
    put(&mut bytes, 0x400, &names);
    let mut dynamic = vec![
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, 4 * 24),
        (11, 24),
    ];
    for (index, (module, library)) in scopes.into_iter().enumerate() {
        let id = 2 + index as u64 * 2;
        let major = if index == 0 && matches!(scope, Scope::WrongModuleVersion) {
            2
        } else {
            1
        };
        let version = if index == 0 && matches!(scope, Scope::WrongLibraryVersion) {
            2
        } else {
            1
        };
        dynamic.push((0x6100_000f, module | (1 << 32) | (major << 40) | (id << 48)));
        dynamic.push((0x6100_0015, library | (version << 32) | ((id + 1) << 48)));
    }
    dynamic.extend([(7, 0x800), (8, 3 * 24), (9, 24), (0, 0)]);
    for at in [152, 160] {
        put(&mut bytes, at, &((dynamic.len() * 16) as u64).to_le_bytes());
    }
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}
fn run(source: &Input) -> (i32, HashMap<String, String>, String) {
    probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "extended-startup".into(),
    ])
}

impl Code {
    fn flags(&mut self) {
        self.bytes.extend([0x48, 0x39, 0xc0, 0xf9]);
    }
    fn check_flags(&mut self) {
        for condition in [0x83, 0x85, 0x8b, 0x88, 0x80] {
            self.bytes.extend([0x0f, condition, 0, 0, 0, 0]);
            self.failed.push(self.bytes.len() - 4);
        }
    }
}
fn initialize(p: &mut Code, insufficient: bool) {
    for offset in [0, 8, 72, 80, 120, 360, 368] {
        p.word(offset, CANARY);
    }
    for offset in (BUFFER..360).step_by(8) {
        p.word(offset, CANARY);
    }
    for (field, target) in [
        (16, BUFFER),
        (24, 360),
        (32, BUFFER),
        (40, if insufficient { 132 } else { 356 }),
        (56, USERDATA),
        (USERDATA + 8, DESCRIPTOR),
        (USERDATA + 48, TEXT),
    ] {
        p.pointer_field(field, target);
    }
    p.path(0, CALLBACK);
    p.stack(0x89, 0, 48);
    p.word(64, (CANARY & !0xffff_ffff) | 1);
    p.word(USERDATA, 0);
    p.call(2);
    p.stack(0x89, 0, USERDATA + 16);
    p.bytes.extend([0xc7, 0, 0x37, 0, 0, 0]);
}
fn text_words(p: &mut Code, bytes: &[u8; 24], check: bool) {
    for (index, chunk) in bytes.chunks_exact(8).enumerate() {
        let value = u64::from_le_bytes(chunk.try_into().unwrap());
        if check {
            p.check(TEXT + index as u32 * 8, value);
        } else {
            p.word(TEXT + index as u32 * 8, value);
        }
    }
}
fn arguments(p: &mut Code, kind: Kind, case: &Case, color: u64) {
    text_words(p, &case.source(), false);
    p.ptr(7, DESCRIPTOR);
    if kind.push() {
        if case.text.is_some() {
            p.ptr(6, TEXT);
        } else {
            p.reg(6, 0);
        }
    } else {
        p.reg(6, u64::MAX);
    }
    p.reg(2, color);
}
fn check_arguments(p: &mut Code, kind: Kind, case: &Case, color: u64) {
    p.same_ptr(7, DESCRIPTOR);
    if kind.push() {
        if case.text.is_some() {
            p.same_ptr(6, TEXT);
        } else {
            p.expect(6, 0);
        }
    } else {
        p.expect(6, u64::MAX);
    }
    p.expect(2, color);
    p.check(0, CANARY);
}
fn check_packet(p: &mut Code, offset: u32, words: &[u32]) {
    for (index, pair) in words.chunks(2).enumerate() {
        let high = pair
            .get(1)
            .map_or(CANARY & !0xffff_ffff, |word| u64::from(*word) << 32);
        p.check(offset + index as u32 * 8, u64::from(pair[0]) | high);
    }
}
fn check_descriptor(p: &mut Code, used: u32, down: u32, calls: u64, grow: bool) {
    for (field, target) in [
        (16, BUFFER),
        (24, 360),
        (32, BUFFER + used),
        (40, down),
        (56, USERDATA),
        (USERDATA + 8, DESCRIPTOR),
        (USERDATA + 48, TEXT),
    ] {
        p.field_is_pointer(field, target);
    }
    if grow {
        p.stack(0x8b, 0, 48);
        p.path(10, CALLBACK);
        p.bytes.extend([0x4c, 0x39, 0xd0]);
        p.bad();
    } else {
        p.check(48, 0);
    }
    p.check(64, (CANARY & !0xffff_ffff) | 1);
    for offset in [8, 72, 80, 120, 360, 368] {
        p.check(offset, CANARY);
    }
    p.check(USERDATA, calls);
    for offset in (BUFFER + used..360).step_by(8) {
        p.check(offset, CANARY);
    }
}
fn fit(kind: Kind, scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.seed_context();
    p.hold_x87();
    let mut used = 0;
    for (index, case) in kind.cases().into_iter().enumerate() {
        let color = 0xfedcba9800000000 | ((index as u64) * 0x1020304);
        arguments(&mut p, kind, &case, color);
        p.flags();
        p.call(0);
        p.check_flags();
        p.same_ptr(0, BUFFER + used);
        check_arguments(&mut p, kind, &case, color);
        check_packet(&mut p, BUFFER + used, &case.words);
        used += case.words.len() as u32 * 4;
        p.field_is_pointer(32, BUFFER + used);
        text_words(&mut p, &case.source(), true);
    }
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, used, 356, 0, true);
    image(&p.finish(42), &[0x0f, 0x0b], kind, scope)
}
fn callback(kind: Kind, request: u64, grows: bool, accepts: bool) -> Vec<u8> {
    let mut p = Code::new(CALLBACK);
    p.stack(0x89, 7, 0);
    p.stack(0x89, 2, 8);
    p.bytes.extend([0x48, 0x8b, 0x42, 8, 0x48, 0x39, 0xc7]);
    p.bad();
    p.expect(6, request);
    p.bytes.extend([
        0x48, 0x89, 0x7a, 24, 0x48, 0x89, 0x72, 32, 0x48, 0x89, 0x52, 40, 0x48, 0xff, 0x02,
    ]);
    if kind.push() {
        // Mutate bytes only AFTER the producer captured the original length.
        p.bytes.extend([0x48, 0x8b, 0x42, 48]);
        p.reg(10, u64::from_le_bytes(kind.replacement()));
        p.bytes.extend([0x4c, 0x89, 0x10]);
    }
    p.reg(7, 3);
    p.call(1);
    p.expect(0, 12);
    p.call(2);
    p.stack(0x8b, 2, 8);
    p.bytes.extend([0x4c, 0x8b, 0x52, 16, 0x4c, 0x39, 0xd0]);
    p.bad();
    p.bytes.extend([0x83, 0x38, 0x37]);
    p.bad();
    p.stack(0x8b, 7, 0);
    if grows {
        p.bytes.extend([0x48, 0x8d, 0x87]);
        p.bytes.extend((356_u32 - DESCRIPTOR).to_le_bytes());
        p.bytes.extend([0x48, 0x89, 0x47, 24]);
    }
    p.seed_context();
    p.finish(0x5a5a5a5a5a5a0000 | u64::from(accepts))
}
fn growth(kind: Kind, scope: Scope, grows: bool, accepts: bool) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    let color = u64::MAX;
    arguments(&mut p, kind, &case, color);
    p.flags();
    p.call(0);
    p.check_flags();
    let used = if grows && accepts {
        p.same_ptr(0, BUFFER);
        check_packet(&mut p, BUFFER, &kind.grown_words());
        case.words.len() as u32 * 4
    } else {
        p.expect(0, 0);
        0
    };
    check_arguments(&mut p, kind, &case, color);
    let mut after = case.source();
    if kind.push() {
        after[..8].copy_from_slice(&kind.replacement());
    }
    text_words(&mut p, &after, true);
    p.check_context(CALLBACK);
    p.check_x87();
    check_descriptor(&mut p, used, if grows { 356 } else { 132 }, 1, true);
    p.field_is_pointer(USERDATA + 24, DESCRIPTOR);
    p.check(USERDATA + 32, case.words.len() as u64 + 1);
    p.field_is_pointer(USERDATA + 40, USERDATA);
    image(
        &p.finish(42),
        &callback(kind, case.words.len() as u64 + 1, grows, accepts),
        kind,
        scope,
    )
}
fn no_authority(kind: Kind) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.word(48, 0);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, kind, &case, u64::MAX);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, kind, &case, u64::MAX);
    text_words(&mut p, &case.source(), true);
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, 0, 132, 0, false);
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn null_buffer(kind: Kind) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    p.reg(7, 0);
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    p.expect(7, 0);
    p.expect(6, u64::MAX);
    p.expect(2, u64::MAX);
    p.check_context(ENTRY);
    p.check_x87();
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn tail_string(kind: Kind) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.seed_context();
    p.hold_x87();
    p.ptr(7, DESCRIPTOR);
    p.path(6, 0xbffe);
    p.reg(2, u64::MAX);
    p.flags();
    p.call(0);
    p.check_flags();
    p.same_ptr(0, BUFFER);
    check_packet(&mut p, BUFFER, &[0xc000102c, 0x62]);
    p.path(10, 0xbffe);
    p.bytes.extend([0x4c, 0x39, 0xd6]);
    p.bad();
    p.same_ptr(7, DESCRIPTOR);
    p.expect(2, u64::MAX);
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, 8, 356, 0, true);
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn no_submission(fields: &HashMap<String, String>, text: &str) {
    for key in [
        "graphics_command_buffers",
        "graphics_draw_attempts",
        "graphics_dispatch_attempts",
        "graphics_written_bytes",
        "window_frames_presented",
        "window_frames_acknowledged",
    ] {
        assert_eq!(fields[key], "0", "{key}: {text}");
    }
}
fn returned(source: &Input, services: u64, graphics: u64) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], services.to_string(), "{text}");
    assert_eq!(fields["graphics_calls"], graphics.to_string(), "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    no_submission(&fields, &text);
}
#[test]
fn native_agc_marker_encoders_copy_exact_padded_bytes_and_preserve_pre_growth_length_with_new_source()
 {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        for kind in [Kind::AcbPush, Kind::DcbPush, Kind::AcbPop, Kind::DcbPop] {
            let calls = kind.cases().len() as u64;
            returned(&fit(kind, scope), calls + 1, calls);
            count += 1;
        }
    }
    for kind in [Kind::AcbPush, Kind::DcbPush, Kind::AcbPop, Kind::DcbPop] {
        returned(&growth(kind, Scope::Observed, true, true), 4, 2);
        count += 1;
        for (grows, accepts) in [(true, false), (false, true)] {
            returned(&growth(kind, Scope::Observed, grows, accepts), 4, 2);
            count += 1;
        }
        returned(&null_buffer(kind), 1, 1);
        count += 1;
    }
    for kind in [Kind::AcbPush, Kind::DcbPop] {
        returned(&no_authority(kind), 2, 1);
        count += 1;
    }
    for kind in [Kind::AcbPush, Kind::DcbPush] {
        returned(&tail_string(kind), 2, 1);
        count += 1;
    }
    assert_eq!(count, 28);
    println!(
        "agc_marker_native_available=true agc_marker_native_programs={count} agc_marker_native_returns={count}"
    );
}

fn string_fault(kind: Kind) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    arguments(&mut p, kind, &case, u64::MAX);
    p.reg(6, u64::MAX);
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn direct_failure(kind: Kind, scope: Scope, null: bool) -> Input {
    let mut p = Code::new(ENTRY);
    p.reg(7, if null { 0 } else { u64::MAX });
    p.reg(6, if null || !kind.push() { u64::MAX } else { 0 });
    p.reg(2, u64::MAX);
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], kind, scope)
}
fn readonly_output(kind: Kind) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.path(0, ENTRY);
    p.stack(0x89, 0, 32);
    p.path(0, ENTRY + case.words.len() as i32 * 4 + 4);
    p.stack(0x89, 0, 40);
    arguments(&mut p, kind, &case, u64::MAX);
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn refused(source: &Input, services: u64) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service", "{text}");
    assert_eq!(fields["runtime_failure"], "graphics-memory", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["service_stops"], services.to_string(), "{text}");
    assert_eq!(fields["graphics_calls"], "1", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    no_submission(&fields, &text);
}
#[test]
fn native_agc_marker_encoders_refuse_unreadable_strings_descriptors_outputs_and_unobserved_scopes()
{
    let mut count = 0;
    for kind in [Kind::AcbPush, Kind::DcbPush] {
        refused(&string_fault(kind), 2);
        count += 1;
    }
    for kind in [Kind::AcbPush, Kind::DcbPush, Kind::AcbPop, Kind::DcbPop] {
        refused(&direct_failure(kind, Scope::Observed, false), 1);
        count += 1;
    }
    for kind in [Kind::AcbPush, Kind::DcbPop] {
        refused(&readonly_output(kind), 2);
        count += 1;
    }
    let kind = Kind::AcbPush;
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    arguments(&mut p, kind, &case, u64::MAX);
    p.call(0);
    let source = image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed);
    let (status, fields, text) = run(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["service_stops"], "2", "{text}");
    assert_eq!(fields["graphics_calls"], "1", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    no_submission(&fields, &text);
    count += 1;
    for scope in [
        Scope::WrongPair,
        Scope::WrongModuleVersion,
        Scope::WrongLibraryVersion,
    ] {
        let (status, fields, text) = run(&direct_failure(Kind::AcbPush, scope, true));
        assert_eq!(status, 2, "scope={scope:?}: {text}");
        if matches!(scope, Scope::WrongPair) {
            assert_eq!(fields["code"], "unavailable-function", "{text}");
            assert_eq!(fields["phase"], "main-entry", "{text}");
            assert_eq!(fields["service_stops"], "1", "{text}");
            assert_eq!(fields["unavailable_functions"], "1", "{text}");
            assert_eq!(fields["graphics_calls"], "0", "{text}");
        } else {
            assert_eq!(fields["code"], "import-version-mismatch", "{text}");
            assert_eq!(fields["phase"], "admission", "{text}");
            assert_eq!(fields["native_instructions"], "0", "{text}");
            assert_eq!(fields["service_stops"], "0", "{text}");
            assert_eq!(fields["unavailable_functions"], "unknown", "{text}");
        }
        assert!(!fields.contains_key("value"), "{text}");
        count += 1;
    }
    assert_eq!(count, 12);
    println!("agc_marker_native_refusal_programs={count} agc_marker_native_refusals={count}");
}
