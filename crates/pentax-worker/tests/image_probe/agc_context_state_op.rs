// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers of source-defined context-state operation command encoders.
//! Packet encoding does not submit commands or establish submitted context-state effects.
use super::*;

const ENTRY: i32 = 0x1000;
const CALLBACK: i32 = 0x6000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const DESCRIPTOR: u32 = 64;
const BUFFER_END: u32 = 640;
const BUFFER: u32 = 256;
const USERDATA: u32 = 664;
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
    fn check_context(&mut self, environment_base: i32) {
        for register in GPRS {
            // RCX/R11 are real syscall clobbers. The actual call arguments are
            // checked separately; R10 is this original consumer's scratch.
            if matches!(register, 1 | 11 | 7 | 6) {
                continue;
            }
            self.expect(register, gpr_pattern(register) ^ self.base as u64);
        }
        for register in 0..16 {
            self.vector(false, register);
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.check(1136 + i as u32 * 8, value ^ ((self.base as u64) << 32));
            }
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
        let control = MXCSR | (((environment_base as u32 >> 12) & 3) << 13);
        self.check(1112, (CANARY & !0xffff_ffff) | u64::from(control));
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
enum Scope {
    Canonical,
    Observed,
    WrongPair,
    WrongModuleVersion,
    WrongLibraryVersion,
}
fn image(main: &[u8], callback: &[u8], scope: Scope) -> Input {
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
    for (index, nid) in ["qj7QZpgr9Uw", "t7PlZ9nt5Lc", "9BcDykPmo1I"]
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
    fn branch(&mut self, condition: u8) -> usize {
        self.bytes.extend([0x0f, condition, 0, 0, 0, 0]);
        self.bytes.len() - 4
    }
    fn jump(&mut self) -> usize {
        self.bytes.extend([0xe9, 0, 0, 0, 0]);
        self.bytes.len() - 4
    }
    fn bind(&mut self, branch: usize) {
        let displacement = self.bytes.len() as i32 - (branch as i32 + 4);
        self.bytes[branch..branch + 4].copy_from_slice(&displacement.to_le_bytes());
    }
    fn descriptor_ptr(&mut self, register: u8, displacement: u32) {
        self.bytes.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            0x8d,
            0x87 | ((register & 7) << 3),
        ]);
        self.bytes.extend(displacement.to_le_bytes());
    }
    fn finish_from_slot(mut self, slot: u32) -> Vec<u8> {
        self.stack(0x8b, 0, slot);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 8, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x5000);
        self.bytes
    }
}
fn packet(operation: u32) -> Vec<u32> {
    match operation {
        0 => vec![0xc0031068, 0x0, 0x0, 0x0, 0x0],
        1 => vec![
            0xc0031068, 0x1, 0x0, 0x0, 0x0, 0xc0061000, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0xc0071000, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0xc0011000, 0x0, 0x0, 0xc0001000,
            0x0,
        ],
        2 => vec![
            0xc0011068, 0x2, 0x0, 0xc0031000, 0x0, 0x0, 0x0, 0x0, 0xc0061000, 0x0, 0x0, 0x0, 0x0,
            0x0, 0x0, 0x0, 0xc0071000, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0xc0001000, 0x0,
        ],
        3 => vec![
            0xc0031068, 0x3, 0x0, 0x0, 0x0, 0xc0061000, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
            0xc0071000, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0xc0011000, 0x0, 0x0, 0xc0001000,
            0x0, 0xc0031000, 0x0, 0x0, 0x0, 0x0,
        ],
        _ => unreachable!("original fixture operation"),
    }
}
#[derive(Clone, Copy)]
struct Step {
    before_dw: u32,
    capacity_dw: u32,
}
fn steps(operation: u32) -> Vec<Step> {
    match operation {
        0 => vec![Step {
            before_dw: 0,
            capacity_dw: 5,
        }],
        1 => vec![
            Step {
                before_dw: 0,
                capacity_dw: 22,
            },
            Step {
                before_dw: 22,
                capacity_dw: 3,
            },
            Step {
                before_dw: 25,
                capacity_dw: 2,
            },
        ],
        2 => vec![
            Step {
                before_dw: 0,
                capacity_dw: 3,
            },
            Step {
                before_dw: 3,
                capacity_dw: 22,
            },
            Step {
                before_dw: 25,
                capacity_dw: 2,
            },
        ],
        3 => vec![
            Step {
                before_dw: 0,
                capacity_dw: 22,
            },
            Step {
                before_dw: 22,
                capacity_dw: 3,
            },
            Step {
                before_dw: 25,
                capacity_dw: 2,
            },
            Step {
                before_dw: 27,
                capacity_dw: 5,
            },
        ],
        _ => unreachable!("original fixture operation"),
    }
}
#[derive(Clone, Copy)]
enum Stop {
    Complete,
    False(usize),
    TooSmall(usize),
}
impl Stop {
    fn at(self) -> Option<usize> {
        match self {
            Self::Complete => None,
            Self::False(i) | Self::TooSmall(i) => Some(i),
        }
    }
    fn accepts(self, index: usize) -> bool {
        !matches!(self, Self::False(i) if i == index)
    }
    fn capacity(self, index: usize, requested: u32) -> u32 {
        if matches!(self, Self::TooSmall(i) if i == index) {
            0
        } else {
            requested
        }
    }
}
fn initialize(p: &mut Code, insufficient: bool) {
    for offset in [48, 56, 120, 128, 248, BUFFER_END, BUFFER_END + 8] {
        p.word(offset, CANARY);
    }
    for offset in (BUFFER..BUFFER_END).step_by(8) {
        p.word(offset, CANARY);
    }
    for (field, target) in [
        (DESCRIPTOR, BUFFER),
        (DESCRIPTOR + 8, BUFFER_END),
        (DESCRIPTOR + 16, BUFFER),
        (
            DESCRIPTOR + 24,
            if insufficient {
                BUFFER + 4
            } else {
                BUFFER_END - 4
            },
        ),
        (DESCRIPTOR + 40, USERDATA),
        (USERDATA + 8, DESCRIPTOR),
    ] {
        p.pointer_field(field, target);
    }
    p.path(0, CALLBACK);
    p.stack(0x89, 0, DESCRIPTOR + 32);
    p.word(DESCRIPTOR + 48, (CANARY & !0xffff_ffff) | 1);
    p.word(USERDATA, 0);
    p.call(2);
    p.stack(0x89, 0, USERDATA + 16);
    p.bytes.extend([0xc7, 0, 0x37, 0, 0, 0]);
}
fn arguments(p: &mut Code, operation: u64) {
    p.ptr(7, DESCRIPTOR);
    p.reg(6, operation);
}
fn check_arguments(p: &mut Code, operation: u64) {
    p.same_ptr(7, DESCRIPTOR);
    p.expect(6, operation);
}
fn check_packet(p: &mut Code, offset: u32, words: &[u32]) {
    for (index, pair) in words.chunks_exact(2).enumerate() {
        p.check(
            offset + index as u32 * 8,
            u64::from(pair[0]) | (u64::from(pair[1]) << 32),
        );
    }
    if !words.len().is_multiple_of(2) {
        p.check(
            offset + (words.len() as u32 - 1) * 4,
            (CANARY & !0xffff_ffff) | u64::from(words[words.len() - 1]),
        );
    }
}
fn check_descriptor(
    p: &mut Code,
    used: u32,
    down: u32,
    callbacks: u64,
    grow: bool,
    numeric_cursor: Option<u64>,
) {
    for (field, target) in [
        (DESCRIPTOR, BUFFER),
        (DESCRIPTOR + 8, BUFFER_END),
        (DESCRIPTOR + 24, down),
        (DESCRIPTOR + 40, USERDATA),
        (USERDATA + 8, DESCRIPTOR),
    ] {
        p.field_is_pointer(field, target);
    }
    if let Some(cursor) = numeric_cursor {
        p.check(DESCRIPTOR + 16, cursor);
    } else {
        p.field_is_pointer(DESCRIPTOR + 16, BUFFER + used);
    }
    if grow {
        p.stack(0x8b, 0, DESCRIPTOR + 32);
        p.path(10, CALLBACK);
        p.bytes.extend([0x4c, 0x39, 0xd0]);
        p.bad();
    } else {
        p.check(DESCRIPTOR + 32, 0);
    }
    p.check(DESCRIPTOR + 48, (CANARY & !0xffff_ffff) | 1);
    p.check(USERDATA, callbacks);
    for offset in [48, 56, 120, 128, 248, BUFFER_END, BUFFER_END + 8] {
        p.check(offset, CANARY);
    }
    for offset in (BUFFER + used..BUFFER_END - 7).step_by(8) {
        p.check(offset, CANARY);
    }
    if !used.is_multiple_of(8) {
        p.check(BUFFER_END - 8, CANARY);
    }
}
fn fit(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.seed_context();
    p.hold_x87();
    let mut used = 0;
    for operation in 0_u32..4 {
        let argument = 0xfeed_face_0000_0000 | u64::from(operation);
        let words = packet(operation);
        arguments(&mut p, argument);
        p.flags();
        p.call(0);
        p.check_flags();
        p.same_ptr(0, BUFFER + used);
        check_arguments(&mut p, argument);
        check_packet(&mut p, BUFFER + used, &words);
        used += words.len() as u32 * 4;
        p.field_is_pointer(DESCRIPTOR + 16, BUFFER + used);
    }
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, used, BUFFER_END - 4, 0, true, None);
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn callback_prefix(p: &mut Code, words: &[u32], before: u32) {
    p.bytes.extend([0x48, 0x8b, 0x47, 16]);
    p.descriptor_ptr(10, BUFFER - DESCRIPTOR + before * 4);
    p.bytes.extend([0x4c, 0x39, 0xd0]);
    p.bad();
    for (index, pair) in words[..before as usize].chunks_exact(2).enumerate() {
        p.descriptor_ptr(0, BUFFER - DESCRIPTOR + index as u32 * 8);
        p.bytes.extend([0x48, 0x8b, 0]);
        p.expect(0, u64::from(pair[0]) | (u64::from(pair[1]) << 32));
    }
    if !before.is_multiple_of(2) {
        p.descriptor_ptr(0, BUFFER - DESCRIPTOR + (before - 1) * 4);
        p.bytes.extend([0x48, 0x8b, 0]);
        p.expect(
            0,
            (CANARY & !0xffff_ffff) | u64::from(words[before as usize - 1]),
        );
    }
    p.descriptor_ptr(0, BUFFER - DESCRIPTOR + before * 4);
    p.bytes.extend([0x48, 0x8b, 0]);
    p.expect(0, CANARY);
}
fn callback(operation: u32, stop: Stop, migrate: bool) -> Vec<u8> {
    let words = packet(operation);
    let mut p = Code::new(CALLBACK);
    p.stack(0x89, 7, 0);
    p.stack(0x89, 2, 8);
    p.bytes.extend([0x48, 0x8b, 0x42, 8, 0x48, 0x39, 0xc7]);
    p.bad();
    p.bytes.extend([0x48, 0xff, 0x02]);
    let mut done = Vec::new();
    for (index, step) in steps(operation).into_iter().enumerate() {
        p.bytes.extend([0x48, 0x8b, 0x02]);
        p.reg(10, index as u64 + 1);
        p.bytes.extend([0x4c, 0x39, 0xd0]);
        let next = p.branch(0x85);
        p.expect(6, u64::from(step.capacity_dw) + 1);
        if migrate && index == 2 {
            // The third callback sees the second segment at its new actual address.
            p.bytes.extend([0x48, 0x8b, 0x47, 16]);
            p.descriptor_ptr(10, BUFFER - DESCRIPTOR + 140);
            p.bytes.extend([0x4c, 0x39, 0xd0]);
            p.bad();
            for (i, pair) in words[..22].chunks_exact(2).enumerate() {
                p.descriptor_ptr(0, BUFFER - DESCRIPTOR + i as u32 * 8);
                p.bytes.extend([0x48, 0x8b, 0]);
                p.expect(0, u64::from(pair[0]) | (u64::from(pair[1]) << 32));
            }
            p.descriptor_ptr(0, BUFFER - DESCRIPTOR + 128);
            p.bytes.extend([0x48, 0x8b, 0]);
            p.expect(0, 0xc001_1000);
            p.descriptor_ptr(0, BUFFER - DESCRIPTOR + 136);
            p.bytes.extend([0x48, 0x8b, 0]);
            p.expect(0, CANARY & !0xffff_ffff);
            p.descriptor_ptr(0, BUFFER - DESCRIPTOR + 140);
            p.bytes.extend([0x48, 0x8b, 0]);
            p.expect(0, CANARY);
        } else {
            callback_prefix(&mut p, &words, step.before_dw);
        }
        p.bytes.extend([
            0x48, 0x89, 0x7a, 24, 0x48, 0x89, 0x72, 32, 0x48, 0x89, 0x52, 40,
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
        if migrate && index == 1 {
            p.descriptor_ptr(0, BUFFER - DESCRIPTOR + 128);
            p.bytes.extend([0x48, 0x89, 0x47, 16]);
        }
        p.bytes.extend([0x48, 0x8b, 0x47, 16, 0x48, 0x05]);
        p.bytes
            .extend(((stop.capacity(index, step.capacity_dw) + 1) * 4).to_le_bytes());
        p.bytes.extend([0x48, 0x89, 0x47, 24]);
        p.word(16, 0x5a5a_5a5a_5a5a_0000 | u64::from(stop.accepts(index)));
        done.push(p.jump());
        p.bind(next);
    }
    p.bytes.extend([0x0f, 0x0b]); // Any unexpected extra callback is a genuine guest failure.
    for branch in done {
        p.bind(branch);
    }
    p.seed_context();
    p.finish_from_slot(16)
}
fn growth(operation: u32, stop: Stop) -> Input {
    let words = packet(operation);
    let plan = steps(operation);
    let last = stop.at().unwrap_or(plan.len() - 1);
    let step = plan[last];
    let used_dw = if stop.at().is_some() {
        step.before_dw
    } else {
        words.len() as u32
    };
    let down = BUFFER + (step.before_dw + stop.capacity(last, step.capacity_dw) + 1) * 4;
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    let argument = 0x1234_5678_0000_0000 | u64::from(operation);
    arguments(&mut p, argument);
    p.flags();
    p.call(0);
    p.check_flags();
    if stop.at().is_none() {
        p.same_ptr(0, BUFFER);
    } else {
        p.expect(0, 0);
    }
    check_arguments(&mut p, argument);
    check_packet(&mut p, BUFFER, &words[..used_dw as usize]);
    p.check_context(CALLBACK);
    p.check_x87();
    check_descriptor(&mut p, used_dw * 4, down, last as u64 + 1, true, None);
    p.field_is_pointer(USERDATA + 24, DESCRIPTOR);
    p.check(USERDATA + 32, u64::from(step.capacity_dw) + 1);
    p.field_is_pointer(USERDATA + 40, USERDATA);
    image(
        &p.finish(42),
        &callback(operation, stop, false),
        Scope::Observed,
    )
}

fn migrated_growth() -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, 1);
    p.flags();
    p.call(0);
    p.check_flags();
    p.same_ptr(0, BUFFER);
    check_arguments(&mut p, 1);
    let words = packet(1);
    check_packet(&mut p, BUFFER, &words[..22]);
    check_packet(&mut p, BUFFER + 128, &words[22..]);
    for offset in (BUFFER + 88..BUFFER + 128).step_by(8) {
        p.check(offset, CANARY);
    }
    p.check_context(CALLBACK);
    p.check_x87();
    check_descriptor(&mut p, 148, BUFFER + 152, 3, true, None);
    p.field_is_pointer(USERDATA + 24, DESCRIPTOR);
    p.check(USERDATA + 32, 3);
    p.field_is_pointer(USERDATA + 40, USERDATA);
    image(
        &p.finish(42),
        &callback(1, Stop::Complete, true),
        Scope::Observed,
    )
}
fn null_and_invalid(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    for operation in 0_u32..4 {
        let argument = 0xfedc_ba98_0000_0000 | u64::from(operation);
        p.reg(7, 0);
        p.reg(6, argument);
        p.flags();
        p.call(0);
        p.check_flags();
        p.expect(0, 0);
        p.expect(7, 0);
        p.expect(6, argument);
    }
    // Invalid low32 operations return before even reading a deliberately bad descriptor.
    for argument in [4_u64, 0xffff_ffff_ffff_ffff] {
        p.reg(7, u64::MAX);
        p.reg(6, argument);
        p.flags();
        p.call(0);
        p.check_flags();
        p.expect(0, 0);
        p.expect(7, u64::MAX);
        p.expect(6, argument);
    }
    p.check_context(ENTRY);
    p.check_x87();
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn no_growth_authority(operation: u32, pop_prefix: bool) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.word(DESCRIPTOR + 32, 0);
    let (used, down) = if pop_prefix {
        (12, BUFFER + 16)
    } else {
        (0, BUFFER + 4)
    };
    p.pointer_field(DESCRIPTOR + 24, down);
    p.seed_context();
    p.hold_x87();
    let argument = 0xabcd_ef01_0000_0000 | u64::from(operation);
    arguments(&mut p, argument);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, argument);
    check_packet(&mut p, BUFFER, &packet(operation)[..used as usize / 4]);
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, used, down, 0, false, None);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
}
fn null_allocation(operation: u32) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.word(DESCRIPTOR + 16, 0);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, u64::from(operation));
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, u64::from(operation));
    p.check_context(ENTRY);
    p.check_x87();
    // AllocateDW advances its null cursor before returning null; no output is authorized.
    let advanced = if operation == 2 { 12 } else { 20 };
    check_descriptor(&mut p, 0, BUFFER_END - 4, 0, true, Some(advanced));
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
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
fn native_agc_context_state_op_consumes_fragmented_packets_real_grow_and_partial_prefixes() {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        returned(&fit(scope), 5, 4);
        count += 1;
        returned(&null_and_invalid(scope), 6, 6);
        count += 1;
    }
    for operation in 0_u32..4 {
        let calls = steps(operation).len() as u64;
        returned(&growth(operation, Stop::Complete), 2 + 2 * calls, 1 + calls);
        count += 1;
        for index in 0..steps(operation).len() {
            let calls = index as u64 + 1;
            for stop in [Stop::False(index), Stop::TooSmall(index)] {
                returned(&growth(operation, stop), 2 + 2 * calls, 1 + calls);
                count += 1;
            }
        }
        returned(&no_growth_authority(operation, false), 2, 1);
        count += 1;
        returned(&null_allocation(operation), 2, 1);
        count += 1;
    }
    returned(&no_growth_authority(2, true), 2, 1);
    count += 1;
    returned(&migrated_growth(), 8, 4);
    count += 1;
    assert_eq!(count, 40);
    println!(
        "agc_context_state_op_native_available=true agc_context_state_op_native_programs={count} agc_context_state_op_native_returns={count}"
    );
}
fn direct_failure(scope: Scope, operation: u32) -> Input {
    let mut p = Code::new(ENTRY);
    p.reg(7, u64::MAX);
    p.reg(6, u64::from(operation));
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn readonly_output(operation: u32) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.path(0, ENTRY);
    p.stack(0x89, 0, DESCRIPTOR + 16);
    p.path(0, ENTRY + 132);
    p.stack(0x89, 0, DESCRIPTOR + 24);
    arguments(&mut p, u64::from(operation));
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
}
fn refused(source: &Input, services: u64) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service", "{text}");
    assert_eq!(fields["runtime_failure"], "graphics-memory", "{text}");
    assert_eq!(fields["service_stops"], services.to_string(), "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["graphics_calls"], "1", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    no_submission(&fields, &text);
}
#[test]
fn native_agc_context_state_op_refuses_actual_memory_callback_and_exact_scope_failures() {
    let mut count = 0;
    for operation in 0_u32..4 {
        refused(&direct_failure(Scope::Observed, operation), 1);
        count += 1;
        refused(&readonly_output(operation), 2);
        count += 1;
        let mut p = Code::new(ENTRY);
        initialize(&mut p, true);
        arguments(&mut p, u64::from(operation));
        p.call(0);
        let source = image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "unsupported-instruction", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        assert_eq!(fields["graphics_calls"], "1", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        no_submission(&fields, &text);
        count += 1;
    }
    for scope in [
        Scope::WrongPair,
        Scope::WrongModuleVersion,
        Scope::WrongLibraryVersion,
    ] {
        let source = direct_failure(scope, 0);
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
    assert_eq!(count, 15);
    println!(
        "agc_context_state_op_native_refusal_programs={count} agc_context_state_op_native_refusals={count}"
    );
}
