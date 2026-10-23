// SPDX-License-Identifier: GPL-2.0-only
//! Original WaitSafe producer and real cooperative-submit callers.
//! Encoding is not a presentation; pending resumption requires an owned ack.
use super::*;

const ENTRY: i32 = 0x1000;
const CALLBACK: i32 = 0x8000;
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

const DATA: i32 = 0x14000;
const CHILD: i32 = 0xc000;
const WAIT: i32 = 0;
const NOP_SIZE: i32 = 1;
const ERRNO: i32 = 2;
const SUBMIT: i32 = 3;
const VIDEO_OPEN: i32 = 4;
const VIDEO_REGISTER: i32 = 5;
const VIDEO_FLIP: i32 = 6;
const VIDEO_PENDING: i32 = 7;
const MAP: i32 = 8;
const CREATE: i32 = 9;
const YIELD: i32 = 10;
const GETPID: i32 = 11;

#[derive(Clone, Copy)]
enum Scope {
    Canonical,
    Observed,
    WrongPair,
    WrongModuleVersion,
    WrongLibraryVersion,
}
fn image(main: &[u8], callback: &[u8], child: &[u8], scope: Scope) -> Input {
    let kernel_nid = |service| {
        pentax_kernel::Service::registrations()
            .iter()
            .find(|row| row.service == service)
            .unwrap()
            .nid
    };
    let imports = [
        ("MWiElSNE8j8", 0),
        ("t7PlZ9nt5Lc", 1),
        ("9BcDykPmo1I", 4),
        ("b4fpgH5ZXxQ", 2),
        ("Up36PTk687E", 3),
        ("rKBUtgRrtbk", 3),
        ("U46NwOiJpys", 3),
        ("zgXifHT9ErY", 3),
        ("IWIBBdTHit4", 4),
        (kernel_nid(pentax_kernel::Service::PthreadCreate), 4),
        (kernel_nid(pentax_kernel::Service::PthreadYield), 4),
        (kernel_nid(pentax_kernel::Service::Getpid), 4),
    ];
    let pair = match scope {
        Scope::Canonical => ("Graphics5", "Graphics5"),
        Scope::WrongPair => ("libSceAgc", "Graphics5"),
        _ => ("libSceAgc", "libSceAgc"),
    };
    let pairs = [
        pair,
        ("Graphics5", "Graphics5"),
        ("Graphics5Driver", "Graphics5Driver"),
        ("VideoOut", "VideoOut"),
        ("libkernel", "libkernel"),
    ];
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x18000, 0);
    bytes[0x400..0x1000].fill(0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x10000u64.to_le_bytes());
    put(&mut bytes, 104, &0x10000u64.to_le_bytes());
    put(&mut bytes, 56, &3u16.to_le_bytes());
    for (at, value) in [(176, 1u32), (180, 6u32)] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    for (at, value) in [
        (184, DATA as u64),
        (192, DATA as u64),
        (208, 0x4000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    assert!(
        main.len() < (CALLBACK - ENTRY) as usize
            && callback.len() < (CHILD - CALLBACK) as usize
            && child.len() < 0x4000
    );
    put(&mut bytes, ENTRY as usize, main);
    put(&mut bytes, CALLBACK as usize, callback);
    put(&mut bytes, CHILD as usize, child);
    let mut names = vec![0];
    let mut scope_offsets = Vec::new();
    for (module, library) in pairs {
        let m = names.len() as u64;
        names.extend(module.as_bytes());
        names.push(0);
        let l = names.len() as u64;
        names.extend(library.as_bytes());
        names.push(0);
        scope_offsets.push((m, l));
    }
    let import_count = imports.len();
    for (index, (nid, group)) in imports.into_iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        let m = char::from(b'C' + group * 2);
        let l = char::from(b'D' + group * 2);
        names.extend(format!("{nid}#{l}#{m}\0").as_bytes());
        put(
            &mut bytes,
            0x800 + index * 24,
            &(0xf00u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x808 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(0x400 + names.len() < 0x600 && 0x600 + (import_count + 1) * 24 <= 0x800);
    put(&mut bytes, 0x400, &names);
    let mut dynamic = vec![
        (5i64, 0x400u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, (import_count as u64 + 1) * 24),
        (11, 24),
    ];
    for (index, (module, library)) in scope_offsets.into_iter().enumerate() {
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
    dynamic.extend([(7, 0x800), (8, import_count as u64 * 24), (9, 24), (0, 0)]);
    for at in [152, 160] {
        put(&mut bytes, at, &((dynamic.len() * 16) as u64).to_le_bytes());
    }
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}
fn run(source: &Input, window: bool) -> (i32, HashMap<String, String>, String) {
    probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "extended-startup".into(),
        "--present-window".into(),
        (if window { "yes" } else { "no" }).into(),
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
    p.call(ERRNO);
    p.stack(0x89, 0, USERDATA + 16);
    p.bytes.extend([0xc7, 0, 0x37, 0, 0, 0]);
}
fn arguments(p: &mut Code, handle: u64, index: u64) {
    p.ptr(7, DESCRIPTOR);
    p.reg(6, handle);
    p.reg(2, index);
}
fn packet(p: &mut Code, offset: u32, handle: u64, index: u64) {
    p.check(offset, (u64::from(handle as u32) << 32) | 0xc005_1018);
    p.check(offset + 8, u64::from(index as u32));
    p.check(offset + 16, 0);
    p.check(offset + 24, CANARY & !0xffff_ffff);
}
fn fit(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, false);
    p.seed_context();
    p.hold_x87();
    let mut used = 0;
    for (handle, index) in [
        (1, 0),
        (0, 0xffff_fffe),
        (u64::MAX, u64::MAX),
        (0x1234_5678_0000_0002, 0x9876_5432_0000_000f),
        (1 << 32, 1 << 32),
    ] {
        arguments(&mut p, handle, index);
        p.flags();
        p.call(WAIT);
        p.check_flags();
        p.same_ptr(0, BUFFER + used);
        packet(&mut p, BUFFER + used, handle, index);
        p.expect(6, handle);
        p.expect(2, index);
        used += 28;
        p.field_is_pointer(DESCRIPTOR + 16, BUFFER + used);
    }
    p.check_context(ENTRY);
    p.check_x87();
    p.check(USERDATA, 0);
    for offset in [48, 56, 120, 128, 248, BUFFER_END, BUFFER_END + 8] {
        p.check(offset, CANARY);
    }
    image(&p.finish(42), &[0x0f, 0x0b], &[], scope)
}
fn callback(capacity: bool, accepts: bool, migrate: bool) -> Vec<u8> {
    let mut p = Code::new(CALLBACK);
    p.stack(0x89, 7, 0);
    p.stack(0x89, 2, 8);
    p.bytes.extend([0x48, 0x8b, 0x42, 8, 0x48, 0x39, 0xc7]);
    p.bad();
    p.expect(6, 8);
    p.bytes.extend([0x48, 0xff, 0x02]);
    p.reg(7, 3);
    p.call(NOP_SIZE);
    p.expect(0, 12);
    p.call(ERRNO);
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
    let mut p = Code::new(ENTRY);
    initialize(&mut p, true);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, 0x1111_1111_ffff_ffff, 0x2222_2222_ffff_fffe);
    p.flags();
    p.call(WAIT);
    p.check_flags();
    let start = if migrate { BUFFER + 128 } else { BUFFER };
    p.same_ptr(0, start);
    packet(&mut p, start, u64::MAX, 0xffff_fffe);
    p.field_is_pointer(DESCRIPTOR + 16, start + 28);
    p.check_context(CALLBACK);
    p.check_x87();
    p.check(USERDATA, 1);
    image(
        &p.finish(42),
        &callback(capacity, accepts, migrate),
        &[],
        Scope::Observed,
    )
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
#[test]
fn native_wait_safe_encodes_exact_low32_fields_and_real_grow_without_waiting_at_the_producer() {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        let (status, fields, text) = run(&fit(scope), false);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        no_submission(&fields, &text);
        count += 1;
    }
    for migrate in [false, true] {
        let (status, fields, text) = run(&growth(true, true, migrate), false);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        no_submission(&fields, &text);
        count += 1;
    }
    assert_eq!(count, 4);
    println!(
        "agc_wait_safe_encoder_native_available=true agc_wait_safe_encoder_native_programs=4 agc_wait_safe_encoder_native_returns=4"
    );
}
#[test]
fn native_wait_safe_producer_refuses_null_allocation_memory_and_false_scope_without_a_fake_packet()
{
    let mut cases = Vec::new();
    for case in 0..7 {
        let mut p = Code::new(ENTRY);
        initialize(&mut p, true);
        match case {
            0 => p.reg(7, 0),
            1 => p.reg(7, 1),
            2 => {
                p.ptr(7, DESCRIPTOR);
                p.word(DESCRIPTOR + 32, 0);
            }
            3 | 4 | 5 => p.ptr(7, DESCRIPTOR),
            _ => {
                p.ptr(7, DESCRIPTOR);
                p.word(DESCRIPTOR + 16, 0);
            }
        }
        p.reg(6, 1);
        p.reg(2, 0);
        p.call(WAIT);
        let cb = if case == 5 {
            vec![0x0f, 0x0b]
        } else {
            callback(case == 3, case == 4, false)
        };
        let scope = Scope::Observed;
        cases.push((
            image(&p.finish(42), &cb, &[], scope),
            if case == 1 {
                "graphics-memory"
            } else {
                "graphics-service"
            },
            case == 5,
        ));
    }
    let mut count = 0;
    for (source, reason, callback_stop) in cases {
        let (status, fields, text) = run(&source, false);
        assert_eq!(status, 2, "{text}");
        if callback_stop {
            assert_eq!(fields["code"], "unsupported-instruction", "{text}");
        } else {
            assert_eq!(fields["code"], "runtime-service", "{text}");
            assert_eq!(fields["runtime_failure"], reason, "{text}");
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
        p.reg(7, 0);
        p.call(WAIT);
        let (status, fields, text) = run(&image(&p.finish(42), &[], &[], scope), false);
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
    assert_eq!(count, 10);
    println!(
        "agc_wait_safe_encoder_native_refusal_programs=10 agc_wait_safe_encoder_native_refusals=10"
    );
}

const MAPPING: u32 = 1328;
const SUBMIT_DESCRIPTOR: u32 = 1640;
const SUBMIT_CONTEXT: u32 = 1664;
const COMMAND_OFFSET: u32 = 0x200;
const PREFIX_OFFSET: u32 = 0x18000;
impl Code {
    fn mapped(&mut self, register: u8, offset: u32) {
        self.stack(0x8b, register, MAPPING);
        if offset != 0 {
            self.bytes
                .extend([0x48 | u8::from(register >= 8), 0x81, 0xc0 | (register & 7)]);
            self.bytes.extend(offset.to_le_bytes());
        }
    }
    fn store_eax(&mut self, offset: u32) {
        self.bytes.extend([0x89, 0x87]);
        self.bytes.extend(offset.to_le_bytes());
    }
    fn mapped_value(&mut self, offset: u32, expected: u32) {
        self.mapped(0, offset);
        self.bytes.extend([0x8b, 0]);
        self.expect(0, u64::from(expected));
    }
}
fn map_and_open(p: &mut Code) {
    p.word(MAPPING, 0);
    p.ptr(7, MAPPING);
    p.reg(6, 0x40000);
    p.reg(2, 0x33);
    p.reg(1, 0);
    p.call(MAP);
    p.expect(0, 0);
    p.reg(7, 255);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, 0);
    p.call(VIDEO_OPEN);
    p.expect(0, 1);
    // Genuine GPU-readable root commands, independently mapped from source RX.
    for (field, offset) in [
        (DESCRIPTOR, COMMAND_OFFSET),
        (DESCRIPTOR + 8, 0x800),
        (DESCRIPTOR + 16, COMMAND_OFFSET + 20),
        (DESCRIPTOR + 24, 0x800),
    ] {
        p.mapped(0, offset);
        p.stack(0x89, 0, field);
    }
    p.word(DESCRIPTOR + 32, 0);
    p.word(DESCRIPTOR + 40, 0);
    p.word(DESCRIPTOR + 48, 1);
}
fn write_command(p: &mut Code, at: u32, target: u32, value: u32) {
    p.mapped(7, COMMAND_OFFSET);
    for (offset, value) in [(at, 0xc003_3700u32), (at + 4, 0x100), (at + 16, value)] {
        p.reg(0, u64::from(value));
        p.store_eax(offset);
    }
    p.mapped(0, target);
    p.store_eax(at + 8);
    p.bytes.extend([0x48, 0xc1, 0xe8, 32]);
    p.store_eax(at + 12);
}
fn command_arguments(p: &mut Code, dwords: u64) {
    p.mapped(0, COMMAND_OFFSET);
    p.stack(0x89, 0, SUBMIT_DESCRIPTOR);
    p.word(SUBMIT_DESCRIPTOR + 8, dwords);
    p.word(SUBMIT_CONTEXT, 0);
    p.ptr(7, SUBMIT_CONTEXT);
    p.ptr(6, SUBMIT_DESCRIPTOR);
}
fn no_pending(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    map_and_open(&mut p);
    write_command(&mut p, 0, PREFIX_OFFSET, 0xfeed_beef);
    for buffer in [(-2i32) as u64, 15] {
        arguments(&mut p, 1, buffer);
        p.call(WAIT);
    }
    write_command(&mut p, 76, PREFIX_OFFSET + 4, 0xcafe_babe);
    p.seed_context();
    p.hold_x87();
    command_arguments(&mut p, 24);
    p.reg(10, CANARY);
    p.flags();
    p.call(SUBMIT);
    p.stack(0x89, 10, 1704); // Snapshot scratch before any assertion can reuse it.
    p.check_flags();
    p.expect(0, 0);
    p.check(1704, CANARY);
    p.same_ptr(7, SUBMIT_CONTEXT);
    p.same_ptr(6, SUBMIT_DESCRIPTOR);
    p.expect(2, gpr_pattern(2) ^ ENTRY as u64);
    p.check_context(ENTRY);
    p.check_x87();
    p.mapped_value(PREFIX_OFFSET, 0xfeed_beef);
    p.mapped_value(PREFIX_OFFSET + 4, 0xcafe_babe);
    p.reg(7, 1);
    p.reg(6, 0);
    p.call(VIDEO_PENDING);
    p.expect(0, 0);
    image(&p.finish(42), &[], &[], scope)
}
#[test]
fn native_wait_safe_no_pending_stream_uses_actual_cooperative_submit_and_preserves_retained_context()
 {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        let (status, fields, text) = run(&no_pending(scope), false);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["graphics_command_buffers"], "1", "{text}");
        for key in [
            "graphics_draw_attempts",
            "graphics_dispatch_attempts",
            "window_frames_presented",
            "window_frames_acknowledged",
        ] {
            assert_eq!(fields[key], "0", "{key}: {text}");
        }
        count += 1;
    }
    assert_eq!(count, 2);
    println!(
        "agc_wait_safe_submit_native_available=true agc_wait_safe_submit_native_programs=2 agc_wait_safe_submit_native_returns=2"
    );
}

fn register_cpu_frame(p: &mut Code) {
    for offset in (1440..1520).step_by(8) {
        p.word(offset, 0);
    }
    p.word(1448, 64u64 << 32);
    p.word(1456, 64);
    p.word(1472, 0x8000_0000_2200_0000);
    p.mapped(0, 0x10000);
    p.stack(0x89, 0, 1560);
    for offset in [1568, 1576, 1584] {
        p.word(offset, 0);
    }
    p.word(0, 0);
    p.word(8, 0);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 0);
    p.ptr(1, 1560);
    p.reg(8, 1);
    p.ptr(9, 1440);
    p.call(VIDEO_REGISTER);
    p.expect(0, 0);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 1);
    p.reg(1, 77);
    p.call(VIDEO_FLIP);
    p.expect(0, 0);
}
fn parked_peer() -> Input {
    let mut p = Code::new(ENTRY);
    map_and_open(&mut p);
    register_cpu_frame(&mut p);
    write_command(&mut p, 0, PREFIX_OFFSET, 0xfeed_beef);
    arguments(&mut p, 1, 0);
    p.call(WAIT);
    write_command(&mut p, 48, PREFIX_OFFSET + 4, 0xcafe_babe);
    p.stack(0x8b, 0, MAPPING);
    p.stack(0x89, 0, 1760);
    p.call(ERRNO);
    p.stack(0x89, 0, 1768);
    p.bytes.extend([0xc7, 0, 55, 0, 0, 0]);
    p.ptr(7, 1720);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.ptr(1, 1760);
    p.reg(8, 0);
    p.call(CREATE);
    p.expect(0, 0);
    p.seed_context();
    p.hold_x87();
    command_arguments(&mut p, 17);
    p.flags();
    p.call(SUBMIT);
    // No legitimate consumer exists in this no-window program. Reaching this
    // deliberate UD2 would indicate a fabricated acknowledgement or early return.
    p.bytes.extend([0x0f, 0x0b]);
    let main = p.finish(42);
    let mut child = Code::new(CHILD);
    child.bytes.extend([0x48, 0x8b, 7]);
    child.stack(0x89, 0, MAPPING);
    child.bytes.extend([0x48, 0x8b, 0x47, 8]);
    child.stack(0x89, 0, 1792);
    child.call(ERRNO);
    child.stack(0x89, 0, 1800);
    child.bytes.extend([0xc7, 0, 41, 0, 0, 0]);
    child.seed_context();
    child.hold_x87();
    let mut ready = Vec::new();
    for _ in 0..8 {
        child.mapped(0, PREFIX_OFFSET);
        child.bytes.extend([0x81, 0x38]);
        child.bytes.extend(0xfeed_beefu32.to_le_bytes());
        child.bytes.extend([0x0f, 0x84]);
        ready.push(child.bytes.len());
        child.bytes.extend([0; 4]);
        child.call(YIELD);
    }
    child.bytes.extend([0x0f, 0x0b]);
    let destination = child.bytes.len();
    for branch in ready {
        child.bytes[branch..branch + 4]
            .copy_from_slice(&((destination - branch - 4) as i32).to_le_bytes());
    }
    child.check_context(CHILD);
    child.check_x87();
    child.hold_x87();
    child.mapped_value(PREFIX_OFFSET + 4, 0);
    child.reg(7, 1);
    child.reg(6, 0);
    child.call(VIDEO_PENDING);
    child.expect(0, 1);
    child.call(GETPID);
    child.expect(0, 1);
    child.call(YIELD);
    child.check_context(CHILD);
    child.check_x87();
    child.mapped_value(PREFIX_OFFSET, 0xfeed_beef);
    child.mapped_value(PREFIX_OFFSET + 4, 0);
    child.call(ERRNO);
    child.stack(0x8b, 10, 1792);
    child.bytes.extend([0x4c, 0x39, 0xd0, 0x0f, 0x84]);
    child.failed.push(child.bytes.len());
    child.bytes.extend([0; 4]);
    child.stack(0x8b, 10, 1800);
    child.bytes.extend([0x4c, 0x39, 0xd0]);
    child.bad();
    child.bytes.extend([0x8b, 0]);
    child.expect(0, 41);
    // Distinct typed endpoint: failed assertions are UD2, never HLT. This proves
    // actual peer progress while the original caller is still parked; no ack or
    // successful outer return is claimed by this source consumer.
    child.bytes.push(0xf4);
    image(&main, &[], &child.finish(77), Scope::Observed)
}
#[test]
fn native_wait_safe_parks_the_submitter_and_keeps_real_peer_progress_without_a_false_ack() {
    let (status, fields, text) = run(&parked_peer(), false);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction", "{text}");
    assert_eq!(fields["instruction"], "Hlt", "{text}");
    assert_eq!(fields["phase"], "thread-entry", "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "1", "{text}");
    assert_eq!(fields["graphics_command_buffers"], "1", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    for key in [
        "graphics_draw_attempts",
        "graphics_dispatch_attempts",
        "window_frames_presented",
        "window_frames_acknowledged",
    ] {
        assert_eq!(fields[key], "0", "{key}: {text}");
    }
    println!("agc_wait_safe_peer_native_programs=1 agc_wait_safe_peer_native_checked_stops=1");
}

// This separate physical consumer needs two genuine Presented acknowledgements.
// Both waits occur inside the one inner submit, itself inside a real grow call.
const PHYSICAL_STAGE: u32 = 0x14000;
const PHYSICAL_COMMANDS: u32 = PHYSICAL_STAGE + 0x200;
const PHYSICAL_LABEL: u32 = 0x21000;
fn physical_unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e00_0000 | (dst << 17) | (op << 9) | src
}
fn physical_binary(op: u32, dst: u32, src: u32, vector: u32) -> u32 {
    (op << 25) | (dst << 17) | (vector << 9) | src
}
fn physical_export(target: u32, registers: [u32; 4]) -> [u32; 2] {
    [
        0xf800_080f | (target << 4),
        registers[0] | (registers[1] << 8) | (registers[2] << 16) | (registers[3] << 24),
    ]
}
fn physical_template() -> (Vec<u8>, Vec<(usize, u32, u8)>, usize) {
    // Original native AMD shader: v5 makes a triangle and the actual PS user
    // SGPRs select RED, then GREEN. There is no substitute host shader/frame.
    let mut vertex = vec![
        physical_binary(0x1a, 0, 129, 5),
        physical_binary(0x1b, 0, 130, 0),
        physical_binary(0x1b, 1, 130, 5),
        physical_unary(6, 0, 256),
        physical_unary(6, 1, 257),
        physical_binary(8, 0, 240, 0),
        physical_binary(8, 1, 240, 1),
        physical_binary(3, 0, 241, 0),
        physical_binary(3, 1, 241, 1),
        physical_binary(3, 0, 0, 0),
        physical_unary(1, 2, 128),
        physical_unary(1, 3, 242),
    ];
    vertex.extend(physical_export(12, [0, 1, 2, 3]));
    vertex.push(0xbf81_0000);
    let mut pixel = vec![
        physical_unary(1, 0, 0),
        physical_unary(1, 1, 1),
        physical_unary(1, 2, 2),
        physical_unary(1, 3, 242),
    ];
    pixel.extend(physical_export(0, [0, 1, 2, 3]));
    pixel.push(0xbf81_0000);
    let mut words = Vec::<u32>::new();
    let set = |words: &mut Vec<u32>, opcode: u32, register: u32, values: &[u32]| {
        words.extend([
            0xc000_0000 | ((values.len() as u32) << 16) | (opcode << 8),
            register,
        ]);
        let at = words.len();
        words.extend(values);
        at
    };
    for register in [
        0x80, 0x1b1, 0x1b3, 0x1b4, 0x1b8, 0x1c4, 0x1e0, 0x200, 0x203, 0x205, 0x207, 0x293, 0x2f8,
        0x31b, 0x31d, 0x31e, 0x31f, 0x321, 0x325, 0x398, 0x3a0, 0x3a8,
    ] {
        set(&mut words, 0x69, register, &[0]);
    }
    for (register, value) in [
        (0x8e, 15),
        (0x8f, 15),
        (0x1b6, 0x8000),
        (0x1c3, 4),
        (0x1c5, 9),
        (0x202, 0x00cc_0010),
        (0x204, 1 << 19),
        (0x206, 0x3f),
        (0x292, 2),
        (0x2d5, 0x0040_0000),
        (0x31c, (10 << 2) | (1 << 15)),
        (0x3b0, (63 << 14) | 63),
        (0x3b8, 0x0106_c000),
    ] {
        set(&mut words, 0x69, register, &[value]);
    }
    let target_low = set(&mut words, 0x69, 0x318, &[0]);
    let target_high = set(&mut words, 0x69, 0x390, &[0]);
    for (left, right, origin) in [
        (0x0c, 0x0d, 0),
        (0x81, 0x82, 0x8000_0000),
        (0x90, 0x91, 0x8000_0000),
        (0x94, 0x95, 0x8000_0000),
    ] {
        set(&mut words, 0x69, left, &[origin]);
        set(&mut words, 0x69, right, &[64 | (64 << 16)]);
    }
    set(
        &mut words,
        0x69,
        0x10f,
        &[
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            1_f32.to_bits(),
            0,
        ],
    );
    let vs = set(&mut words, 0x76, 0xc8, &[0, 0]);
    set(&mut words, 0x76, 0x88, &[0, 0, 0x602c_0000, 0x0003_0002, 0]);
    let ps = set(
        &mut words,
        0x76,
        0x08,
        &[0, 0, 0x002c_0000, 6, 1_f32.to_bits(), 0, 0],
    );
    set(&mut words, 0x79, 0x242, &[4]);
    set(&mut words, 0x79, 0x24a, &[0]);
    words.extend([0xc000_2f00, 1]);
    let mut patches = vec![
        (target_low, 0, 8),
        (target_high, 0, 40),
        (vs, PHYSICAL_STAGE, 8),
        (vs + 1, PHYSICAL_STAGE, 40),
        (ps, PHYSICAL_STAGE + 0x100, 8),
        (ps + 1, PHYSICAL_STAGE + 0x100, 40),
    ];
    for turn in 0..3 {
        let at = words.len();
        words.extend([0xc003_3700, 0x100, 0, 0, 11 * (turn + 1)]);
        patches.extend([
            (at + 2, PHYSICAL_LABEL + turn * 4, 0),
            (at + 3, PHYSICAL_LABEL + turn * 4, 32),
        ]);
        if turn < 2 {
            if turn == 1 {
                set(&mut words, 0x76, 0x0c, &[0, 1_f32.to_bits(), 0]);
            }
            words.extend([0xc001_2d00, 3, 2]);
            words.extend([0xc004_105c, 1, 0, 1, 77 + turn, 0]);
            words.extend([0xc005_1018, 1, 0, 0, 0, 0, 0]);
        }
    }
    let mut template = vec![0; 4096];
    assert!(vertex.len() * 4 <= 0x100 && pixel.len() * 4 <= 0x100 && words.len() * 4 <= 0xe00);
    for (offset, values) in [(0, vertex), (0x100, pixel), (0x200, words.clone())] {
        for (index, value) in values.into_iter().enumerate() {
            put(&mut template, offset + index * 4, &value.to_le_bytes());
        }
    }
    (template, patches, words.len())
}
fn physical_image() -> Input {
    let (template, patches, dwords) = physical_template();
    let mut p = Code::new(ENTRY);
    map_and_open(&mut p);
    // Round above the allocation start, keeping prefix and suffix inside the
    // actual owned mapping. The 64KiB tiled target occupies its first block.
    p.stack(0x8b, 0, MAPPING);
    p.bytes
        .extend([0x48, 0x05, 7, 0, 1, 0, 0x48, 0x25, 0, 0, 0xff, 0xff]);
    p.stack(0x89, 0, MAPPING);
    p.mapped(7, 0);
    p.reg(0, 0x3f2f_1f11_3f2f_1f11);
    p.reg(1, 65536 / 8);
    p.bytes.extend([0xf3, 0x48, 0xab]);
    p.path(6, DATA + 0x1000);
    p.mapped(7, PHYSICAL_STAGE);
    p.reg(1, 4096);
    p.bytes.extend([0xf3, 0xa4]);
    for (word, offset, shift) in patches {
        p.mapped(0, offset);
        p.bytes.extend([0x48, 0xc1, 0xe8, shift]);
        p.mapped(7, PHYSICAL_COMMANDS);
        p.store_eax(word as u32 * 4);
    }
    p.mapped(7, 0);
    p.reg(0, CANARY);
    p.bytes.extend([0x48, 0x89, 0x47, 0xf8]);
    p.mapped(7, 0x10000);
    p.bytes.extend([0x48, 0x89, 7]);
    p.mapped(7, PHYSICAL_LABEL);
    p.reg(0, 0);
    p.bytes.extend([0x48, 0x89, 7, 0x48, 0x89, 0x47, 8]);
    for offset in (1440..1520).step_by(8) {
        p.word(offset, 0);
    }
    p.word(1448, 64_u64 << 32);
    p.word(1456, 64);
    p.word(1472, 0x8000_0000_2200_0000);
    p.mapped(0, 0);
    p.stack(0x89, 0, 1560);
    for offset in [1568, 1576, 1584] {
        p.word(offset, 0);
    }
    p.word(0, 0);
    p.word(8, 0);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 0);
    p.ptr(1, 1560);
    p.reg(8, 1);
    p.ptr(9, 1440);
    p.call(VIDEO_REGISTER);
    p.expect(0, 0);
    p.mapped(0, PHYSICAL_COMMANDS);
    p.stack(0x89, 0, SUBMIT_DESCRIPTOR);
    p.word(SUBMIT_DESCRIPTOR + 8, dwords as u64);
    p.word(SUBMIT_CONTEXT, 0);
    p.stack(0x8b, 0, MAPPING);
    p.stack(0x89, 0, USERDATA);
    p.pointer_field(USERDATA + 8, SUBMIT_DESCRIPTOR);
    p.pointer_field(USERDATA + 16, SUBMIT_CONTEXT);
    p.call(ERRNO);
    p.stack(0x89, 0, USERDATA + 24);
    p.bytes.extend([0xc7, 0, 55, 0, 0, 0]);
    p.word(USERDATA + 32, 0);
    p.pointer_field(USERDATA + 40, DESCRIPTOR);
    for (field, offset) in [
        (DESCRIPTOR, 0x20000),
        (DESCRIPTOR + 8, 0x21000),
        (DESCRIPTOR + 16, 0x20000),
        (DESCRIPTOR + 24, 0x20004),
    ] {
        p.mapped(0, offset);
        p.stack(0x89, 0, field);
    }
    p.path(0, CALLBACK);
    p.stack(0x89, 0, DESCRIPTOR + 32);
    p.pointer_field(DESCRIPTOR + 40, USERDATA);
    p.word(DESCRIPTOR + 48, 1);
    p.seed_context();
    p.hold_x87();
    arguments(&mut p, 1, 0);
    p.reg(10, CANARY);
    p.flags();
    p.call(WAIT);
    p.stack(0x89, 10, 1704);
    p.check_flags();
    // The outer grow call returns the actual allocation only after BOTH waits.
    p.mapped(10, 0x20000);
    p.bytes.extend([0x4c, 0x39, 0xd0]);
    p.bad();
    p.check(1704, CANARY);
    p.same_ptr(7, DESCRIPTOR);
    p.expect(6, 1);
    p.expect(2, 0);
    p.check_context(CALLBACK);
    p.check_x87();
    p.check(USERDATA + 32, 1);
    p.mapped_value(PHYSICAL_LABEL, 11);
    p.mapped_value(PHYSICAL_LABEL + 4, 22);
    p.mapped_value(PHYSICAL_LABEL + 8, 33);
    p.mapped(0, 0);
    p.bytes.extend([0x48, 0x8b, 0x40, 0xf8]);
    p.expect(0, CANARY);
    p.mapped(0, 0x10000);
    p.bytes.extend([0x48, 0x8b, 0]);
    p.expect(0, CANARY);
    p.reg(7, 1);
    p.reg(6, 0);
    p.call(VIDEO_PENDING);
    p.expect(0, 0);
    let main = p.finish(42);
    let mut cb = Code::new(CALLBACK);
    cb.stack(0x89, 7, 0);
    cb.stack(0x89, 2, 8);
    cb.expect(6, 8);
    cb.bytes.extend([0x48, 0x8b, 0x42, 40, 0x48, 0x39, 0xc7]);
    cb.bad();
    cb.bytes.extend([0x48, 0xff, 0x42, 32, 0x48, 0x8b, 2]);
    cb.stack(0x89, 0, MAPPING);
    cb.reg(7, 3);
    cb.call(NOP_SIZE);
    cb.expect(0, 12);
    cb.call(ERRNO);
    cb.stack(0x8b, 2, 8);
    cb.bytes.extend([0x4c, 0x8b, 0x52, 24, 0x4c, 0x39, 0xd0]);
    cb.bad();
    cb.bytes.extend([0x83, 0x38, 55]);
    cb.bad();
    cb.seed_context();
    cb.stack(0x8b, 10, 8);
    cb.bytes.extend([0x49, 0x8b, 0x7a, 16, 0x49, 0x8b, 0x72, 8]);
    cb.reg(10, CANARY);
    cb.flags();
    cb.call(SUBMIT);
    cb.stack(0x89, 10, 1704);
    cb.check_flags();
    cb.expect(0, 0);
    cb.check(1704, CANARY);
    cb.expect(2, gpr_pattern(2) ^ CALLBACK as u64);
    // The actual argument pointers must survive too; RCX/R11 are syscall clobbers.
    cb.stack(0x8b, 10, 8);
    cb.bytes.extend([0x4d, 0x8b, 0x52, 16, 0x4c, 0x39, 0xd7]);
    cb.bad();
    cb.stack(0x8b, 10, 8);
    cb.bytes.extend([0x4d, 0x8b, 0x52, 8, 0x4c, 0x39, 0xd6]);
    cb.bad();
    cb.check_context(CALLBACK);
    cb.check_x87();
    cb.hold_x87();
    cb.call(ERRNO);
    cb.stack(0x8b, 10, 8);
    cb.bytes.extend([0x4d, 0x8b, 0x52, 24, 0x4c, 0x39, 0xd0]);
    cb.bad();
    cb.bytes.extend([0x83, 0x38, 55]);
    cb.bad();
    cb.mapped_value(PHYSICAL_LABEL, 11);
    cb.mapped_value(PHYSICAL_LABEL + 4, 22);
    cb.mapped_value(PHYSICAL_LABEL + 8, 33);
    // Consume actual output bytes, without a detiler/oracle or replacement image.
    cb.mapped(7, 0);
    cb.reg(1, 65536 / 4);
    cb.reg(10, 0xff00_ff00);
    let scan = cb.bytes.len();
    cb.bytes.extend([0x8b, 7, 0x4c, 0x39, 0xd0, 0x0f, 0x84]);
    let found = cb.bytes.len();
    cb.bytes.extend([0; 4]);
    cb.bytes
        .extend([0x48, 0x83, 0xc7, 4, 0x48, 0xff, 0xc9, 0x0f, 0x85]);
    let again = cb.bytes.len();
    cb.bytes
        .extend((scan as i32 - again as i32 - 4).to_le_bytes());
    cb.bytes.extend([0x0f, 0x0b]);
    let end = cb.bytes.len();
    cb.bytes[found..found + 4].copy_from_slice(&((end - found - 4) as i32).to_le_bytes());
    cb.stack(0x8b, 7, 0);
    cb.bytes.extend([
        0x48, 0x8b, 0x47, 16, 0x48, 0x83, 0xc0, 32, 0x48, 0x89, 0x47, 24,
    ]);
    let callback = cb.finish(1);
    let source = image(&main, &callback, &[], Scope::Observed);
    let mut bytes = source.bytes.clone();
    drop(source);
    put(&mut bytes, (DATA + 0x1000) as usize, &template);
    create_input(bytes)
}
#[test]
#[ignore = "requires actual Vulkan, a visible monitor and two real Presented acknowledgements; run alone under an owning 180-second gate"]
fn native_wait_safe_preserves_nested_callback_and_same_submit_token_through_two_real_presentations()
{
    let (status, fields, text) = run(&physical_image(), true);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    for (key, expected) in [
        ("graphics_command_buffers", "1"),
        ("graphics_draw_attempts", "2"),
        ("graphics_dispatch_attempts", "0"),
        ("window_frames_presented", "2"),
        ("window_frames_acknowledged", "2"),
        ("video_flip_acknowledged", "2"),
    ] {
        assert_eq!(fields[key], expected, "{key}: {text}");
    }
    println!(
        "agc_wait_safe_resumed_native_available=true agc_wait_safe_resumed_native_submits=1 agc_wait_safe_resumed_native_draws=2 agc_wait_safe_resumed_native_presentations=2 agc_wait_safe_resumed_native_acknowledgements=2"
    );
}
