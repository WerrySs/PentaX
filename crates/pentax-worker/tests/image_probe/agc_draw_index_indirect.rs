// SPDX-License-Identifier: GPL-2.0-only
//! Original exact packet consumers of indexed indirect draw encoding.
//! These consumers do not execute indirect draw arguments or submit GPU work.
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
            if matches!(register, 1 | 11 | 7 | 6 | 2) {
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
    for (index, nid) in ["t1vNu082-jM", "t7PlZ9nt5Lc", "9BcDykPmo1I"]
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

#[derive(Clone, Copy)]
struct Case {
    args: [u64; 2],
    words: [u32; 5],
}
const CASES: [Case; 12] = [
    Case {
        args: [0, 0],
        words: [0xc0032500, 0, 0x280, 0x280, 0],
    },
    Case {
        args: [1, 0x100],
        words: [0xc0032500, 1, 0x280, 0x280, 0x20],
    },
    Case {
        args: [0xfeed_beef_5566_778f, 0x0000_0001_0000_0100],
        words: [0xc0032500, 0x5566778f, 0x280, 0x280, 0],
    },
    Case {
        args: [u64::MAX, 0x0000_0002_0000_0100],
        words: [0xc0032500, 0xffffffff, 0x280, 0x280, 0x20],
    },
    Case {
        args: [0xabcd_abcd_0000_0004, 0x6018_8207],
        words: [0xc0032500, 4, 0x010e010d, 0x1000010f, 0],
    },
    Case {
        args: [0xaabb_ccdd_0000_0008, 0xa018_8207],
        words: [0xc0032500, 8, 0x010e010d, 0x1000010f, 0],
    },
    Case {
        args: [0x1111_2222_3333_4445, 0x0018_8207],
        words: [0xc0032500, 0x33334445, 0x008e008d, 0x1000008f, 0],
    },
    Case {
        args: [0x4444_5555_6666_7777, 0xe018_8207],
        words: [0xc0032500, 0x66667777, 0x008e008d, 0x1000008f, 0],
    },
    Case {
        args: [0x8888_9999_aaaa_aaab, 1],
        words: [0xc0032500, 0xaaaaaaab, 0x8c, 0x280, 0],
    },
    Case {
        args: [0xbbbb_cccc_dddd_ddde, 2],
        words: [0xc0032500, 0xddddddde, 0x008c0280, 0x10000280, 0],
    },
    Case {
        args: [0xeeee_ffff_1234_5679, 4],
        words: [0xc0032500, 0x12345679, 0x280, 0x8c, 0],
    },
    Case {
        args: [u64::MAX, u64::MAX],
        words: [0xc0032500, 0xffffffff, 0x00ab00ab, 0x100000ab, 0],
    },
];
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
fn arguments(p: &mut Code, case: Case, null: bool) {
    if null {
        p.reg(7, 0);
    } else {
        p.ptr(7, DESCRIPTOR);
    }
    for (register, value) in [6, 2].into_iter().zip(case.args) {
        p.reg(register, value);
    }
}
fn check_arguments(p: &mut Code, case: Case, null: bool) {
    if null {
        p.expect(7, 0);
    } else {
        p.same_ptr(7, DESCRIPTOR);
    }
    p.expect(6, case.args[0]);
    p.expect(2, case.args[1]);
}
fn check_packet(p: &mut Code, offset: u32, case: Case) {
    for (index, pair) in case.words.chunks_exact(2).enumerate() {
        p.check(
            offset + index as u32 * 8,
            u64::from(pair[0]) | (u64::from(pair[1]) << 32),
        );
    }
    p.check(
        offset + 16,
        (CANARY & !0xffff_ffff) | u64::from(case.words[4]),
    );
}
fn check_descriptor(
    p: &mut Code,
    start: u32,
    used: u32,
    down: u32,
    callbacks: u64,
    grow: bool,
    null_cursor: bool,
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
    if null_cursor {
        p.check(DESCRIPTOR + 16, 20);
    } else {
        p.field_is_pointer(DESCRIPTOR + 16, start + used);
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
    for offset in (BUFFER..start).step_by(8) {
        p.check(offset, CANARY);
    }
    for offset in (start + used..BUFFER_END - 7).step_by(8) {
        p.check(offset, CANARY);
    }
    if !(start + used).is_multiple_of(8) {
        p.check(BUFFER_END - 8, CANARY);
    }
}
fn fit(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.seed_context();
    p.hold_x87();
    let mut used = 0;
    for case in CASES {
        arguments(&mut p, case, false);
        p.flags();
        p.call(0);
        p.check_flags();
        p.same_ptr(0, BUFFER + used);
        check_arguments(&mut p, case, false);
        check_packet(&mut p, BUFFER + used, case);
        used += 20;
        p.field_is_pointer(DESCRIPTOR + 16, BUFFER + used);
    }
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, BUFFER, used, BUFFER_END - 4, 0, true, false);
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn null_buffer(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    let case = CASES[11];
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, case, true);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, case, true);
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
    if capacity {
        p.bytes.extend([0x48, 0x8d, 0x87]);
        p.bytes.extend((BUFFER_END - 4 - DESCRIPTOR).to_le_bytes());
        p.bytes.extend([0x48, 0x89, 0x47, 24]);
    }
    if migrate {
        p.bytes.extend([0x48, 0x8d, 0x87]);
        p.bytes.extend((BUFFER + 128 - DESCRIPTOR).to_le_bytes());
        p.bytes.extend([0x48, 0x89, 0x47, 16]);
    }
    p.seed_context();
    p.finish(0x5a5a_5a5a_5a5a_0000 | u64::from(accepts))
}
fn growth(capacity: bool, accepts: bool, migrate: bool) -> Input {
    let case = CASES[11];
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, case, false);
    p.flags();
    p.call(0);
    p.check_flags();
    let start = if migrate { BUFFER + 128 } else { BUFFER };
    let used = if capacity && accepts {
        p.same_ptr(0, start);
        check_packet(&mut p, start, case);
        20
    } else {
        p.expect(0, 0);
        0
    };
    check_arguments(&mut p, case, false);
    p.check_context(CALLBACK);
    p.check_x87();
    check_descriptor(
        &mut p,
        start,
        used,
        if capacity { BUFFER_END - 4 } else { BUFFER + 4 },
        1,
        true,
        false,
    );
    p.field_is_pointer(USERDATA + 24, DESCRIPTOR);
    p.check(USERDATA + 32, 6);
    p.field_is_pointer(USERDATA + 40, USERDATA);
    image(
        &p.finish(42),
        &callback(capacity, accepts, migrate),
        Scope::Observed,
    )
}
fn no_growth_authority() -> Input {
    let case = CASES[11];
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.word(DESCRIPTOR + 32, 0);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, case, false);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, case, false);
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, BUFFER, 0, BUFFER + 4, 0, false, false);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
}
fn null_allocation() -> Input {
    let case = CASES[11];
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.word(DESCRIPTOR + 16, 0);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, case, false);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    check_arguments(&mut p, case, false);
    p.check_context(ENTRY);
    p.check_x87();
    check_descriptor(&mut p, BUFFER, 0, BUFFER_END - 4, 0, true, true);
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
fn native_agc_draw_index_indirect_consumes_literal_modifiers_offsets_grow_and_context() {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        returned(&fit(scope), 13, 12);
        count += 1;
    }
    for migrate in [false, true] {
        returned(&growth(true, true, migrate), 4, 2);
        count += 1;
    }
    assert_eq!(count, 4);
    println!(
        "agc_draw_index_indirect_native_available=true agc_draw_index_indirect_native_programs={count} agc_draw_index_indirect_native_returns={count}"
    );
}
fn direct_failure(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    arguments(&mut p, CASES[11], true);
    p.reg(7, u64::MAX);
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn readonly_output() -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.path(0, ENTRY);
    p.stack(0x89, 0, DESCRIPTOR + 16);
    p.path(0, ENTRY + 24);
    p.stack(0x89, 0, DESCRIPTOR + 24);
    arguments(&mut p, CASES[11], false);
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed)
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
fn native_agc_draw_index_indirect_retains_fatal_allocator_memory_and_exact_scope_refusals() {
    let mut count = 0;
    refused(&direct_failure(Scope::Observed), "graphics-memory", 1, 1);
    count += 1;
    refused(&readonly_output(), "graphics-memory", 2, 1);
    count += 1;
    refused(&null_buffer(Scope::Observed), "graphics-service", 1, 1);
    count += 1;
    refused(&no_growth_authority(), "graphics-service", 2, 1);
    count += 1;
    refused(&null_allocation(), "graphics-service", 2, 1);
    count += 1;
    for (capacity, accepts) in [(true, false), (false, true)] {
        refused(&growth(capacity, accepts, false), "graphics-service", 4, 2);
        count += 1;
    }
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    arguments(&mut p, CASES[11], false);
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
    for scope in [
        Scope::WrongPair,
        Scope::WrongModuleVersion,
        Scope::WrongLibraryVersion,
    ] {
        let source = direct_failure(scope);
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
    assert_eq!(count, 11);
    println!(
        "agc_draw_index_indirect_native_refusal_programs={count} agc_draw_index_indirect_native_refusals={count}"
    );
}
