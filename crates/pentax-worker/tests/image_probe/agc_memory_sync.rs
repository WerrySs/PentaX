// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers of source-defined acquire-memory and event command encoders.
//! Packet encoding does not submit commands or establish GPU synchronization effects.
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
            if matches!(register, 1 | 11 | 7 | 6 | 2 | 8 | 9) {
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

#[derive(Clone, Copy, Debug)]
enum Kind {
    AcbAcquire,
    DcbAcquire,
    AcbEvent,
    DcbEvent,
}
impl Kind {
    fn nid(self) -> &'static str {
        match self {
            Self::AcbAcquire => "KT-hTp-Ch14",
            Self::DcbAcquire => "57labkp+rSQ",
            Self::AcbEvent => "cFazmnXpJOE",
            Self::DcbEvent => "aJf+j5yntiU",
        }
    }
    fn cases(self) -> Vec<Case> {
        match self {
            Self::AcbAcquire => vec![
                Case::acb_acquire(0, 0, 0, 0, [0xc0061050, 0x80000000, 0, 0, 0, 0, 0, 0]),
                Case::acb_acquire(
                    0xdeadbeef,
                    0x0123456789abcdef,
                    0x00aabbccddeeff12,
                    0x123456780000004f,
                    [
                        0xc0061050, 0x80000000, 0xccddeeff, 0, 0x6789abcd, 0, 1, 0xdeadbeef,
                    ],
                ),
                Case::acb_acquire(
                    1,
                    u64::MAX,
                    u64::MAX,
                    39,
                    [0xc0061050, 0x80000000, 0, 0, 0xffffffff, 0, 0, 1],
                ),
                Case::acb_acquire(
                    0x89abcdef12345678,
                    0x10000000001,
                    0x1ff,
                    40,
                    [0xc0061050, 0x80000000, 1, 0, 0, 0, 1, 0x12345678],
                ),
                Case::acb_acquire(
                    u64::MAX,
                    u64::MAX,
                    u64::MAX - 1,
                    u64::MAX,
                    [
                        0xc0061050, 0x80000000, 0xffffffff, 0, 0xffffffff, 0, 0x06666666,
                        0xffffffff,
                    ],
                ),
            ],
            Self::DcbAcquire => vec![
                Case::dcb_acquire(
                    [0, 0x123400, 0x01234567, 0x89abcdef, 0x12345678ff, 0x100],
                    40,
                    [0xc0061050, 0x01234567, 1, 0, 0x12345678, 0, 1, 0x89abcdef],
                ),
                Case::dcb_acquire(
                    [0, 1, 0x01234567, 0x76543210, 0, u64::MAX],
                    39,
                    [0xc0061050, 0x81234567, 0, 0, 0, 0, 0, 0x76543210],
                ),
                Case::dcb_acquire(
                    [0, 2, 0x80000001, 0x123456789abcdef0, u64::MAX, 0xffffffffff],
                    u64::MAX,
                    [
                        0xc0061050, 0x80000001, 0xffffffff, 0, 0xffffffff, 0, 0x06666666,
                        0x9abcdef0,
                    ],
                ),
                Case::dcb_acquire(
                    [0, 0xff, 0x7ffffffe, 0xdeadbeef, 0x10000000001, 0x1ff],
                    79,
                    [0xc0061050, 0xfffffffe, 1, 0, 0, 0, 1, 0xdeadbeef],
                ),
                Case::dcb_acquire(
                    [0, 0x100, u64::MAX, 0, 0, 0],
                    0,
                    [0xc0061050, 0xffffffff, 0, 0, 0, 0, 0, 0],
                ),
            ],
            Self::AcbEvent => vec![
                Case::event(0, u64::MAX, &[0xc0004600, 0]),
                Case::event(7, u64::MAX, &[0xc0004600, 0x407]),
                Case::event(15, u64::MAX, &[0xc0004600, 15]),
                Case::event(16, u64::MAX, &[0xc0004600, 16]),
                Case::event(0x38, u64::MAX, &[0xc0004600, 0x38]),
                Case::event(0x47, u64::MAX, &[0xc0004600, 7]),
                Case::event(0x107, u64::MAX, &[0xc0004600, 0x407]),
                Case::event(u64::MAX, u64::MAX, &[0xc0004600, 0x3f]),
            ],
            Self::DcbEvent => vec![
                Case::event(0, u64::MAX, &[0xc0004600, 0]),
                Case::event(7, u64::MAX, &[0xc0004600, 0x407]),
                Case::event(15, u64::MAX, &[0xc0004600, 0x40f]),
                Case::event(16, u64::MAX, &[0xc0004600, 0x410]),
                Case::event(
                    0x38,
                    0x123456789abcdef7,
                    &[0xc0024600, 0x138, 0x9abcdef0, 0x12345678],
                ),
                Case::event(0x39, u64::MAX, &[0xc0024600, 0x139, 0xfffffff8, 0xffffffff]),
                Case::event(0x3f, u64::MAX, &[0xc0004600, 0x3f]),
                Case::event(0x100, u64::MAX, &[0xc0004600, 0]),
                Case::event(0x138, 7, &[0xc0024600, 0x138, 0, 0]),
            ],
        }
    }
    fn growth_case(self) -> Case {
        match self {
            Self::AcbAcquire | Self::DcbAcquire | Self::AcbEvent => self.cases().remove(1),
            Self::DcbEvent => self.cases().remove(4),
        }
    }
}
struct Case {
    args: [u64; 6],
    stack: u64,
    words: Vec<u32>,
}
impl Case {
    fn acb_acquire(gcr: u64, base: u64, size: u64, poll: u64, words: [u32; 8]) -> Self {
        Self {
            args: [0, gcr, base, size, poll, 0x3141592653589793],
            stack: CANARY,
            words: words.to_vec(),
        }
    }
    fn dcb_acquire(args: [u64; 6], poll: u64, words: [u32; 8]) -> Self {
        Self {
            args,
            stack: poll,
            words: words.to_vec(),
        }
    }
    fn event(event: u64, address: u64, words: &[u32]) -> Self {
        Self {
            args: [
                0,
                event,
                address,
                0x123456789abcdef0,
                0x3141592653589793,
                0x8877665544332211,
            ],
            stack: CANARY,
            words: words.to_vec(),
        }
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
    bytes.resize(0xb000, 0);
    bytes[0x400..0x1000].fill(0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0xb000_u64.to_le_bytes());
    put(&mut bytes, 104, &0xb000_u64.to_le_bytes());
    assert!(main.len() < (CALLBACK - ENTRY) as usize);
    assert!(callback.len() < 0x5000);
    put(&mut bytes, ENTRY as usize, main);
    put(&mut bytes, CALLBACK as usize, callback);
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
    for offset in [8, 72, 80, 120, 360, 368] {
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
fn arguments(p: &mut Code, case: &Case, descriptor: bool) {
    p.word(0, case.stack); // Real SysV seventh argument: callee sees [RSP+8].
    if descriptor {
        p.ptr(7, DESCRIPTOR);
    } else {
        p.reg(7, 0);
    }
    for (register, value) in [6, 2, 1, 8, 9].into_iter().zip(case.args[1..].iter()) {
        p.reg(register, *value);
    }
}
fn check_arguments(p: &mut Code, case: &Case) {
    p.same_ptr(7, DESCRIPTOR);
    // RCX/R11 are the existing real SYSCALL clobbers, not caller-save evidence.
    for (register, index) in [(6, 1), (2, 2), (8, 4), (9, 5)] {
        p.expect(register, case.args[index]);
    }
    p.check(0, case.stack);
}
fn check_packet(p: &mut Code, offset: u32, words: &[u32]) {
    assert!(words.len().is_multiple_of(2));
    for (index, pair) in words.chunks_exact(2).enumerate() {
        p.check(
            offset + index as u32 * 8,
            u64::from(pair[0]) | (u64::from(pair[1]) << 32),
        );
    }
}
fn check_descriptor(p: &mut Code, used_bytes: u32, down: u32, callbacks: u64, grow: bool) {
    for (field, target) in [
        (16, BUFFER),
        (24, 360),
        (32, BUFFER + used_bytes),
        (40, down),
        (56, USERDATA),
        (USERDATA + 8, DESCRIPTOR),
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
    p.check(USERDATA, callbacks);
    for offset in (BUFFER + used_bytes..360).step_by(8) {
        p.check(offset, CANARY);
    }
}
fn fit(kind: Kind, scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.seed_context();
    p.hold_x87();
    let mut used = 0;
    for case in kind.cases() {
        arguments(&mut p, &case, true);
        p.flags();
        p.call(0);
        p.check_flags();
        p.same_ptr(0, BUFFER + used);
        used += case.words.len() as u32 * 4;
        p.field_is_pointer(32, BUFFER + used);
        check_arguments(&mut p, &case);
        check_packet(
            &mut p,
            BUFFER + used - case.words.len() as u32 * 4,
            &case.words,
        );
    }
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, used, 356, 0, true);
    image(&p.finish(42), &[0x0f, 0x0b], kind, scope)
}
fn callback(request: u64, grows: bool, accepts: bool) -> Vec<u8> {
    let mut p = Code::new(CALLBACK);
    p.stack(0x89, 7, 0);
    p.stack(0x89, 2, 8);
    p.bytes.extend([0x48, 0x8b, 0x42, 8, 0x48, 0x39, 0xc7]);
    p.bad();
    p.expect(6, request);
    p.bytes.extend([
        0x48, 0x89, 0x7a, 24, 0x48, 0x89, 0x72, 32, 0x48, 0x89, 0x52, 40, 0x48, 0xff, 0x02,
    ]);
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
    p.seed_context(); // Caller GPR/YMM restore; same-thread MXCSR intentionally persists.
    p.finish(0x5a5a5a5a5a5a0000 | u64::from(accepts))
}
fn growth(kind: Kind, scope: Scope, grows: bool, accepts: bool) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, &case, true);
    p.flags();
    p.call(0);
    p.check_flags();
    let used = if grows && accepts {
        p.same_ptr(0, BUFFER);
        check_packet(&mut p, BUFFER, &case.words);
        case.words.len() as u32 * 4
    } else {
        p.expect(0, 0);
        0
    };
    check_arguments(&mut p, &case);
    p.check_context(CALLBACK);
    p.check_x87();
    check_descriptor(&mut p, used, if grows { 356 } else { 132 }, 1, true);
    p.field_is_pointer(USERDATA + 24, DESCRIPTOR);
    p.check(USERDATA + 32, case.words.len() as u64 + 1);
    p.field_is_pointer(USERDATA + 40, USERDATA);
    image(
        &p.finish(42),
        &callback(case.words.len() as u64 + 1, grows, accepts),
        kind,
        scope,
    )
}
fn no_growth_authority(kind: Kind) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.word(48, 0);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, &case, true);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, &case);
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, 0, 132, 0, false);
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn null_event(scope: Scope) -> Input {
    let case = Kind::AcbEvent.growth_case();
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, &case, false);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    p.check_context(ENTRY);
    p.check_x87();
    p.check(0, case.stack);
    image(&p.finish(42), &[0x0f, 0x0b], Kind::AcbEvent, scope)
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
fn native_agc_memory_sync_encoders_consume_exact_source_packets_stack_args_and_real_growth() {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        for kind in [
            Kind::AcbAcquire,
            Kind::DcbAcquire,
            Kind::AcbEvent,
            Kind::DcbEvent,
        ] {
            let calls = kind.cases().len() as u64;
            returned(&fit(kind, scope), calls + 1, calls);
            count += 1;
        }
        returned(&null_event(scope), 1, 1);
        count += 1;
    }
    for kind in [
        Kind::AcbAcquire,
        Kind::DcbAcquire,
        Kind::AcbEvent,
        Kind::DcbEvent,
    ] {
        returned(&growth(kind, Scope::Observed, true, true), 4, 2);
        count += 1;
    }
    for kind in [Kind::AcbAcquire, Kind::DcbAcquire, Kind::AcbEvent] {
        for (grows, accepts) in [(true, false), (false, true)] {
            returned(&growth(kind, Scope::Observed, grows, accepts), 4, 2);
            count += 1;
        }
    }
    for kind in [Kind::AcbAcquire, Kind::AcbEvent] {
        returned(&no_growth_authority(kind), 2, 1);
        count += 1;
    }
    assert_eq!(count, 22);
    println!(
        "agc_memory_sync_native_available=true agc_memory_sync_native_programs={count} agc_memory_sync_native_returns={count}"
    );
}

fn direct_failure(kind: Kind, descriptor: u64, event: Option<u64>, scope: Scope) -> Input {
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    arguments(&mut p, &case, false);
    p.reg(7, descriptor);
    if let Some(event) = event {
        p.reg(6, event);
    }
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
    arguments(&mut p, &case, true);
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], kind, Scope::Observed)
}
fn refused(source: &Input, failure: &str, services: u64, graphics: u64) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service", "{text}");
    assert_eq!(fields["runtime_failure"], failure, "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["service_stops"], services.to_string(), "{text}");
    assert_eq!(fields["graphics_calls"], graphics.to_string(), "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    no_submission(&fields, &text);
}
#[test]
fn native_agc_memory_sync_refuses_exact_fatal_inputs_faults_and_scope_versions_without_fake_packets()
 {
    let mut count = 0;
    for kind in [Kind::AcbAcquire, Kind::DcbAcquire, Kind::DcbEvent] {
        refused(
            &direct_failure(kind, 0, None, Scope::Observed),
            "graphics-service",
            1,
            1,
        );
        count += 1;
    }
    for event in [0x40, 0xff] {
        // The low-byte event predicate runs before reading this bad descriptor.
        refused(
            &direct_failure(Kind::DcbEvent, u64::MAX, Some(event), Scope::Observed),
            "graphics-service",
            1,
            1,
        );
        count += 1;
    }
    for kind in [
        Kind::AcbAcquire,
        Kind::DcbAcquire,
        Kind::AcbEvent,
        Kind::DcbEvent,
    ] {
        refused(
            &direct_failure(kind, u64::MAX, None, Scope::Observed),
            "graphics-memory",
            1,
            1,
        );
        count += 1;
    }
    for kind in [Kind::AcbAcquire, Kind::DcbEvent] {
        refused(&readonly_output(kind), "graphics-memory", 2, 1);
        count += 1;
    }
    for (grows, accepts) in [(true, false), (false, true)] {
        // DCB EventWrite is source-fatal here, unlike AcquireMem and ACB events.
        refused(
            &growth(Kind::DcbEvent, Scope::Observed, grows, accepts),
            "graphics-service",
            4,
            2,
        );
        count += 1;
    }
    refused(
        &no_growth_authority(Kind::DcbEvent),
        "graphics-service",
        2,
        1,
    );
    count += 1;
    let kind = Kind::DcbAcquire;
    let case = kind.growth_case();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    arguments(&mut p, &case, true);
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
        let source = direct_failure(Kind::AcbAcquire, 0, None, scope);
        let (status, fields, text) = run(&source);
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
    assert_eq!(count, 18);
    println!(
        "agc_memory_sync_native_refusal_programs={count} agc_memory_sync_native_refusals={count}"
    );
}
