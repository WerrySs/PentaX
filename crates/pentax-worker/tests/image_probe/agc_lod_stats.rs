// SPDX-License-Identifier: GPL-2.0-only
//! Original exact LOD-statistics producer callers; encoding grants no feedback authority.
//! No submitted counter results or commercial frame are inferred from these callers.
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
            if matches!(register, 1 | 11 | 7 | 6 | 2 | 8 | 9) {
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
    for (index, nid) in ["vuSXe69VILM", "t7PlZ9nt5Lc", "9BcDykPmo1I"]
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

// Five register arguments after buf plus two actual outgoing SysV stack slots.
// First16framebytes are disjoint from the descriptor at64 and all canaries.
#[derive(Clone, Copy)]
struct Case {
    args: [u64; 7],
    words: [u32; 5],
}
const CASES: [Case; 7] = [
    Case {
        args: [0, 0, 0, 0, 0, 0, 0],
        words: [0xc0038e00, 0, 0, 0, 0],
    },
    Case {
        args: [u64::MAX; 7],
        words: [0xc0038e00, 0xffffffff, 0xffffffc0, 0xffffffff, 0x300ffffc],
    },
    Case {
        args: [
            0x1122334400000102,
            0x11223344556677bf,
            0xaabbccdd00000041,
            0x9999000000001234,
            0x100,
            0x101,
            0x102,
        ],
        words: [0xc0038e00, 0x41, 0x55667780, 0x11223344, 0x2008d008],
    },
    Case {
        args: [
            1,
            0x123456780000003f,
            4,
            0x80,
            0x101,
            0x100,
            0x8000000000000101,
        ],
        words: [0xc0038e00, 4, 0, 0x12345678, 0x10060004],
    },
    Case {
        args: [0x100000000, 1, 3, 0x1000, 0, 0, 0x100000000],
        words: [0xc0038e00, 3, 0, 0, 0],
    },
    Case {
        args: [
            3,
            0xabcdef0187654340,
            0x100000000,
            0x101,
            0xffff00,
            0xffff00,
            0x1ff,
        ],
        words: [0xc0038e00, 0, 0x87654340, 0xabcdef01, 0x300007fc],
    },
    Case {
        args: [2, 0x100000000, 1, 0, 1, 0, 0],
        words: [0xc0038e00, 1, 0, 1, 0x20040000],
    },
];
const CHANGED_STACK: [u64; 2] = [0xface000000000000, 0xbeef00000000005a];
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
    p.bytes.extend([0xc7, 0, 55, 0, 0, 0]);
    p.ptr(0, 0);
    p.stack(0x89, 0, USERDATA + 24);
}
fn arguments(p: &mut Code, case: Case, null: bool) {
    if null {
        p.reg(7, 0);
    } else {
        p.ptr(7, DESCRIPTOR);
    }
    for (reg, value) in [6, 2, 1, 8, 9]
        .into_iter()
        .zip(case.args[..5].iter().copied())
    {
        p.reg(reg, value);
    }
    p.word(0, case.args[5]);
    p.word(8, case.args[6]);
}
fn call(p: &mut Code) {
    p.reg(10, CANARY);
    p.flags();
    p.call(0);
    p.stack(0x89, 10, 1600);
    p.check_flags();
}
fn check_arguments(p: &mut Code, case: Case, null: bool, changed: bool) {
    if null {
        p.expect(7, 0);
    } else {
        p.same_ptr(7, DESCRIPTOR);
    }
    // RCX/R11 are the established mediated syscall clobbers; packet literals
    // consume the actual low32 size. All other call arguments remain observable.
    for (reg, index) in [(6, 0), (2, 1), (8, 3), (9, 4)] {
        p.expect(reg, case.args[index]);
    }
    p.check(
        0,
        if changed {
            CHANGED_STACK[0]
        } else {
            case.args[5]
        },
    );
    p.check(
        8,
        if changed {
            CHANGED_STACK[1]
        } else {
            case.args[6]
        },
    );
    p.check(1600, CANARY);
}
fn packet(p: &mut Code, at: u32, words: [u32; 5]) {
    for (index, pair) in words.chunks_exact(2).enumerate() {
        p.check(
            at + index as u32 * 8,
            u64::from(pair[0]) | (u64::from(pair[1]) << 32),
        );
    }
    p.check(at + 16, (CANARY & !0xffff_ffff) | u64::from(words[4]));
}
fn guards(p: &mut Code) {
    for offset in [48, 56, 120, 128, 248, BUFFER_END, BUFFER_END + 8] {
        p.check(offset, CANARY);
    }
}
fn fit(scope: Scope, exact_boundary: bool) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    let start = if exact_boundary {
        BUFFER_END - 24
    } else {
        BUFFER
    };
    if exact_boundary {
        p.pointer_field(DESCRIPTOR + 16, start);
        p.pointer_field(DESCRIPTOR + 24, BUFFER_END);
    }
    p.seed_context();
    p.hold_x87();
    let mut used = 0;
    let cases: &[Case] = if exact_boundary { &CASES[2..3] } else { &CASES };
    for &case in cases {
        arguments(&mut p, case, false);
        call(&mut p);
        p.same_ptr(0, start + used);
        check_arguments(&mut p, case, false, false);
        packet(&mut p, start + used, case.words);
        used += 20;
        p.field_is_pointer(DESCRIPTOR + 16, start + used);
    }
    p.check_context(ENTRY);
    p.check_x87();
    p.check(USERDATA, 0);
    guards(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn null_buffer(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, CASES[1], true);
    call(&mut p);
    p.expect(0, 0);
    check_arguments(&mut p, CASES[1], true, false);
    p.check_context(ENTRY);
    p.check_x87();
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn callback(capacity: bool, accepts: bool, migrate: bool) -> Vec<u8> {
    let mut p = Code::new(CALLBACK);
    p.stack(0x89, 7, 0);
    p.stack(0x89, 2, 8);
    p.bytes.extend([0x48, 0x8b, 0x42, 8, 0x48, 0x39, 0xc7]);
    p.bad();
    p.expect(6, 6);
    p.bytes.extend([0x48, 0xff, 0x02]);
    p.reg(7, 3);
    p.call(1);
    p.expect(0, 12);
    p.call(2);
    p.stack(0x8b, 2, 8);
    p.bytes.extend([0x4c, 0x8b, 0x52, 16, 0x4c, 0x39, 0xd0]);
    p.bad();
    p.bytes.extend([0x83, 0x38, 55]);
    p.bad();
    // Rewrite the real parent's outgoing slots AFTER capture. Correct packet
    // data still comes from the original arguments, never from a second scan.
    p.bytes.extend([0x48, 0x8b, 0x7a, 24]);
    for (offset, value) in [(0, CHANGED_STACK[0]), (8, CHANGED_STACK[1])] {
        p.reg(0, value);
        p.bytes.extend([0x48, 0x89, 0x87]);
        p.bytes.extend((offset as u32).to_le_bytes());
    }
    p.stack(0x8b, 7, 0);
    if migrate {
        p.bytes.extend([0x48, 0x8d, 0x87]);
        p.bytes.extend((BUFFER + 128 - DESCRIPTOR).to_le_bytes());
        p.bytes.extend([0x48, 0x89, 0x47, 16]);
    }
    if capacity {
        p.bytes.extend([
            0x48, 0x8b, 0x47, 16, 0x48, 0x83, 0xc0, 24, 0x48, 0x89, 0x47, 24,
        ]);
    }
    p.seed_context();
    p.finish(0x1234_5678_9abc_0000 | u64::from(accepts))
}
fn growth(capacity: bool, accepts: bool, migrate: bool) -> Input {
    let case = CASES[2];
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, case, false);
    call(&mut p);
    let start = if migrate { BUFFER + 128 } else { BUFFER };
    if capacity && accepts {
        p.same_ptr(0, start);
        packet(&mut p, start, case.words);
        p.field_is_pointer(DESCRIPTOR + 16, start + 20);
    } else {
        p.expect(0, 0);
        p.field_is_pointer(DESCRIPTOR + 16, start);
    }
    check_arguments(&mut p, case, false, true);
    p.check_context(CALLBACK);
    p.check_x87();
    p.check(USERDATA, 1);
    guards(&mut p);
    p.field_is_pointer(
        DESCRIPTOR + 24,
        if capacity { start + 24 } else { BUFFER + 4 },
    );
    if !(capacity && accepts) {
        for offset in (BUFFER..BUFFER_END).step_by(8) {
            p.check(offset, CANARY);
        }
    }
    image(
        &p.finish(42),
        &callback(capacity, accepts, migrate),
        Scope::Observed,
    )
}
fn allocation_failure(null_cursor: bool) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, !null_cursor);
    if null_cursor {
        p.word(DESCRIPTOR + 16, 0);
    } else {
        p.word(DESCRIPTOR + 32, 0);
    }
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, CASES[2], false);
    call(&mut p);
    p.expect(0, 0);
    check_arguments(&mut p, CASES[2], false, false);
    p.check_context(ENTRY);
    p.check_x87();
    p.check(USERDATA, 0);
    if null_cursor {
        p.check(DESCRIPTOR + 16, 20);
    } else {
        p.field_is_pointer(DESCRIPTOR + 16, BUFFER);
    }
    for offset in (BUFFER..BUFFER_END).step_by(8) {
        p.check(offset, CANARY);
    }
    guards(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
}
fn descriptor_alias() -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.pointer_field(DESCRIPTOR + 16, DESCRIPTOR);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, CASES[2], false);
    call(&mut p);
    p.same_ptr(0, DESCRIPTOR);
    check_arguments(&mut p, CASES[2], false, false);
    p.check(DESCRIPTOR, 0x00000041c0038e00);
    p.check(DESCRIPTOR + 8, 0x1122334455667780);
    p.stack(0x8b, 0, DESCRIPTOR + 16);
    p.reg(10, 0xffff_ffff);
    p.bytes.extend([0x4c, 0x21, 0xd0]);
    p.expect(0, 0x2008d008);
    // Allocation publishes cursor+20 before the packet overwrites its low word.
    p.ptr(10, DESCRIPTOR + 20);
    p.bytes.extend([0x49, 0xc1, 0xea, 32]);
    p.bytes.extend([0x8b, 0x84, 0x24]);
    p.bytes.extend((DESCRIPTOR + 20).to_le_bytes());
    p.bytes.extend([0x4c, 0x39, 0xd0]);
    p.bad();
    p.field_is_pointer(DESCRIPTOR + 24, BUFFER_END - 4);
    p.check_context(ENTRY);
    p.check_x87();
    guards(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
}
fn guarded_stack(null: bool, second_slot: bool) -> Input {
    let mut p = Code::new(ENTRY);
    p.stack(0x89, 7, 1720);
    initialize(&mut p, false);
    p.hold_x87();
    arguments(&mut p, CASES[2], null);
    // Initial RDI is the real entry argument area, exactly0x100 below the owned
    // stack end. Only the forthcoming stack argument span reaches its guard;
    // the mediated CALL's own return slot remains in live writable stack.
    p.stack(0x8b, 0, 1720);
    p.bytes.extend([0x48, 0x05, 0, 1, 0, 0]);
    if second_slot {
        p.bytes.extend([0x48, 0x83, 0xe8, 8]);
        p.reg(10, CASES[2].args[5]);
        p.bytes.extend([0x4c, 0x89, 0x10]);
    }
    p.bytes.extend([0x49, 0x89, 0xe4, 0x48, 0x89, 0xc4]);
    p.flags();
    p.call(0);
    p.bytes.extend([0x4c, 0x89, 0xe4]);
    p.check_flags();
    if null {
        p.expect(0, 0);
        p.check_x87();
    } else {
        p.bytes.extend([0x0f, 0x0b]);
    }
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
fn returned(source: &Input) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    no_submission(&fields, &text);
}
#[test]
fn native_agc_lod_stats_consumes_five_words_stack_arguments_and_real_grow_without_feedback_claims()
{
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        returned(&fit(scope, false));
        count += 1;
        returned(&null_buffer(scope));
        count += 1;
    }
    returned(&fit(Scope::Observed, true));
    count += 1;
    for (capacity, accepts, migrate) in [
        (true, true, false),
        (true, true, true),
        (true, false, false),
        (false, true, false),
    ] {
        returned(&growth(capacity, accepts, migrate));
        count += 1;
    }
    for null_cursor in [false, true] {
        returned(&allocation_failure(null_cursor));
        count += 1;
    }
    returned(&descriptor_alias());
    count += 1;
    for second in [false, true] {
        returned(&guarded_stack(true, second));
        count += 1;
    }
    assert_eq!(count, 14);
    println!(
        "agc_lod_stats_native_available=true agc_lod_stats_native_programs=14 agc_lod_stats_native_returns=14"
    );
}
#[test]
fn native_agc_lod_stats_refuses_unreadable_stack_descriptor_output_callback_and_false_scopes() {
    let mut cases = Vec::new();
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    arguments(&mut p, CASES[2], false);
    p.reg(7, 1);
    p.call(0);
    cases.push((
        image(&p.finish(42), &[], Scope::Observed),
        "graphics-memory",
        false,
    ));
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.path(0, ENTRY);
    p.stack(0x89, 0, DESCRIPTOR + 16);
    p.path(0, ENTRY + 24);
    p.stack(0x89, 0, DESCRIPTOR + 24);
    arguments(&mut p, CASES[2], false);
    p.call(0);
    cases.push((
        image(&p.finish(42), &[], Scope::Observed),
        "graphics-memory",
        false,
    ));
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    arguments(&mut p, CASES[2], false);
    p.call(0);
    cases.push((
        image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed),
        "",
        true,
    ));
    for second in [false, true] {
        cases.push((guarded_stack(false, second), "graphics-memory", false));
    }
    let mut count = 0;
    for (source, expected, callback_stop) in cases {
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "{text}");
        if callback_stop {
            assert_eq!(fields["code"], "unsupported-instruction", "{text}");
            assert_eq!(fields["instruction"], "Ud2", "{text}");
        } else {
            assert_eq!(fields["code"], "runtime-service", "{text}");
            assert_eq!(fields["runtime_failure"], expected, "{text}");
        }
        assert!(!fields.contains_key("value"), "{text}");
        no_submission(&fields, &text);
        count += 1;
    }
    for scope in [
        Scope::WrongPair,
        Scope::WrongModuleVersion,
        Scope::WrongLibraryVersion,
    ] {
        let mut p = Code::new(ENTRY);
        arguments(&mut p, CASES[0], true);
        p.call(0);
        let (status, fields, text) = run(&image(&p.finish(42), &[], scope));
        assert_eq!(status, 2, "{text}");
        assert_eq!(
            fields["code"],
            if matches!(scope, Scope::WrongPair) {
                "unavailable-function"
            } else {
                "import-version-mismatch"
            },
            "{text}"
        );
        assert!(!fields.contains_key("value"), "{text}");
        count += 1;
    }
    assert_eq!(count, 8);
    println!("agc_lod_stats_native_refusal_programs=8 agc_lod_stats_native_refusals=8");
}
