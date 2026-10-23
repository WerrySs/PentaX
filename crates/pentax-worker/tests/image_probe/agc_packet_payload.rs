// SPDX-License-Identifier: GPL-2.0-only
//! Original exact-width consumers of the payload-address metadata helper.
//! Returned pointers are data and do not grant memory access or submit commands.
use super::*;

const ENTRY: i32 = 0x1000;
const CALLBACK: i32 = 0x6000;
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
    UnobservedRange,
}
fn image(main: &[u8], callback: &[u8], scope: Scope, tail_header: u32) -> Input {
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x14000, 0);
    // Keep RX and RW on separate16KiB logical pages. The final RW tail ends
    // below the next64KiB provider placement, leaving a real unmapped gap.
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (offset, value) in [
        (184, 0x10000_u64),
        (192, 0x10000),
        (208, 0x4000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(&mut bytes, offset, &value.to_le_bytes());
    }
    put(&mut bytes, 0x13ffc, &tail_header.to_le_bytes());
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
    for (index, nid) in [
        if matches!(scope, Scope::UnobservedRange) {
            "s+VGAMDQ0AQ"
        } else {
            "V++UgBtQhn0"
        },
        "t7PlZ9nt5Lc",
        "9BcDykPmo1I",
    ]
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

const COMMAND: u32 = 256;
const OUTPUT: u32 = 321; // Deliberately unaligned, with exact adjacent8B canaries.
const TAIL: i32 = 0x13ffc;
fn initialize(p: &mut Code) {
    p.word(COMMAND - 8, CANARY);
    p.word(COMMAND + 8, CANARY);
    p.call(2);
    p.stack(0x89, 0, 64);
    p.bytes.extend([0xc7, 0, 0x37, 0, 0, 0]);
    p.seed_context();
    p.hold_x87();
}
fn output(p: &mut Code) {
    for at in [OUTPUT - 8, OUTPUT, OUTPUT + 8] {
        p.word(at, CANARY);
    }
}
fn check_output_guards(p: &mut Code) {
    p.check(OUTPUT - 8, CANARY);
    p.check(OUTPUT + 8, CANARY);
}
fn check_tls(p: &mut Code) {
    p.call(2);
    p.stack(0x8b, 10, 64);
    p.bytes.extend([0x4c, 0x39, 0xd0]);
    p.bad();
    p.bytes.extend([0x83, 0x38, 0x37]);
    p.bad();
}
fn finish(p: &mut Code) {
    check_tls(p);
    p.check_context(ENTRY);
    p.check_x87();
}
fn headers(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    for header in [
        0_u32,
        0xc000_1000,
        0xc003_1068,
        0x3ffe_0000,
        0x3fff_0000,
        0xffff_ffff,
        0x7fff_1234,
        0x4000_0000,
    ] {
        p.word(COMMAND, (CANARY & !0xffff_ffff) | u64::from(header));
        output(&mut p);
        p.ptr(7, OUTPUT);
        p.ptr(6, COMMAND);
        p.reg(2, 0x1234_5678_0000_0000);
        p.flags();
        p.call(0);
        p.check_flags();
        p.expect(0, 0);
        p.same_ptr(7, OUTPUT);
        p.same_ptr(6, COMMAND);
        p.expect(2, 0x1234_5678_0000_0000);
        if !header & 0x3fff_0000 != 0 {
            p.field_is_pointer(OUTPUT, COMMAND + 4);
        } else {
            p.check(OUTPUT, 0);
        }
        check_output_guards(&mut p);
        p.check(COMMAND, (CANARY & !0xffff_ffff) | u64::from(header));
        p.check(COMMAND - 8, CANARY);
        p.check(COMMAND + 8, CANARY);
    }
    finish(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], scope, 0)
}
fn nonzero(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    for (command, kind) in [
        (0_u64, 1_u64),
        (1, 0xffff_ffff),
        (u64::MAX - 8, 0x8000_0000),
        (0x1234_5678, 0xaaaa_bbbb_0000_0002),
    ] {
        output(&mut p);
        p.ptr(7, OUTPUT);
        p.reg(6, command);
        p.reg(2, kind);
        p.flags();
        p.call(0);
        p.check_flags();
        p.expect(0, 0);
        p.same_ptr(7, OUTPUT);
        p.expect(6, command);
        p.expect(2, kind);
        p.check(OUTPUT, command + 8);
        check_output_guards(&mut p);
    }
    finish(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], scope, 0)
}
fn aliases(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    for displacement in [0_u32, 4] {
        p.word(COMMAND, (CANARY & !0xffff_ffff) | 0xc000_1000);
        p.word(COMMAND + 8, CANARY);
        p.word(COMMAND + 12, CANARY);
        p.ptr(7, COMMAND + displacement);
        p.ptr(6, COMMAND);
        p.reg(2, 0);
        p.flags();
        p.call(0);
        p.check_flags();
        p.expect(0, 0);
        p.same_ptr(7, COMMAND + displacement);
        p.same_ptr(6, COMMAND);
        p.expect(2, 0);
        p.field_is_pointer(COMMAND + displacement, COMMAND + 4);
        p.check(COMMAND - 8, CANARY);
        p.check(COMMAND + 12, CANARY);
        if displacement == 0 {
            p.check(COMMAND + 8, CANARY);
        } else {
            p.stack(0x8b, 0, COMMAND);
            p.bytes.extend([0x89, 0xc0]);
            p.expect(0, 0xc000_1000);
        }
    }
    finish(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], scope, 0)
}
fn boundary(header: u32) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    output(&mut p);
    p.ptr(7, OUTPUT);
    p.path(6, TAIL);
    p.reg(2, 0);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    p.same_ptr(7, OUTPUT);
    p.path(10, TAIL);
    p.bytes.extend([0x4c, 0x39, 0xd6]);
    p.bad();
    p.expect(2, 0);
    p.stack(0x8b, 0, OUTPUT);
    if !header & 0x3fff_0000 != 0 {
        // The result lies in the actual unmapped tail: it is compared, never read.
        p.path(10, TAIL + 4);
        p.bytes.extend([0x4c, 0x39, 0xd0]);
        p.bad();
    } else {
        p.expect(0, 0);
    }
    check_output_guards(&mut p);
    finish(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed, header)
}
fn tail_output() -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    p.path(7, TAIL - 4);
    p.reg(6, 0);
    p.reg(2, 1);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    p.path(0, TAIL - 4);
    p.bytes.extend([0x48, 0x8b, 0]);
    p.expect(0, 8);
    p.expect(6, 0);
    p.expect(2, 1);
    finish(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed, 0)
}
fn readonly_command() -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    output(&mut p);
    p.ptr(7, OUTPUT);
    p.path(6, ENTRY);
    p.reg(2, 0);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    // The original prologue word0xec814855 is deliberately not a packet opcode.
    p.stack(0x8b, 0, OUTPUT);
    p.path(10, ENTRY + 4);
    p.bytes.extend([0x4c, 0x39, 0xd0]);
    p.bad();
    check_output_guards(&mut p);
    finish(&mut p);
    image(&p.finish(42), &[0x0f, 0x0b], Scope::Observed, 0)
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
fn returned(source: &Input, calls: u64) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], (calls + 2).to_string(), "{text}");
    assert_eq!(fields["graphics_calls"], calls.to_string(), "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    no_submission(&fields, &text);
}
#[test]
fn native_agc_packet_payload_address_consumes_branches_exact_width_aliases_and_context() {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        returned(&headers(scope), 8);
        count += 1;
        returned(&nonzero(scope), 4);
        count += 1;
        returned(&aliases(scope), 2);
        count += 1;
    }
    for header in [0xc000_1000, 0x3fff_0000] {
        returned(&boundary(header), 1);
        count += 1;
    }
    returned(&tail_output(), 1);
    count += 1;
    returned(&readonly_command(), 1);
    count += 1;
    assert_eq!(count, 10);
    println!(
        "agc_packet_payload_native_available=true agc_packet_payload_native_programs={count} agc_packet_payload_native_returns={count}"
    );
}
fn failure(scope: Scope, case: u8) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p);
    output(&mut p);
    p.ptr(7, OUTPUT);
    p.reg(6, 0);
    p.reg(2, 1);
    match case {
        0 => p.reg(7, 0),
        1 => p.reg(7, u64::MAX),
        2 => p.path(7, ENTRY),
        3 => p.path(7, TAIL),
        4 => p.reg(2, 0),
        5 => {
            p.reg(2, 0);
            p.reg(6, u64::MAX);
        }
        6 => p.reg(6, u64::MAX),
        _ => unreachable!("original refusal case"),
    }
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], scope, 0)
}
#[test]
fn native_agc_packet_payload_address_refuses_memory_scope_versions_and_unobserved_range() {
    let mut count = 0;
    for case in 0..7 {
        let source = failure(Scope::Observed, case);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "case={case}: {text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], "graphics-memory", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        assert_eq!(fields["graphics_calls"], "1", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        no_submission(&fields, &text);
        count += 1;
    }
    for scope in [
        Scope::WrongPair,
        Scope::WrongModuleVersion,
        Scope::WrongLibraryVersion,
        Scope::UnobservedRange,
    ] {
        let source = failure(scope, 0);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "scope={scope:?}: {text}");
        if matches!(scope, Scope::WrongPair | Scope::UnobservedRange) {
            assert_eq!(fields["code"], "unavailable-function", "{text}");
            assert_eq!(fields["phase"], "main-entry", "{text}");
            assert_eq!(fields["service_stops"], "2", "{text}");
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
        "agc_packet_payload_native_refusal_programs={count} agc_packet_payload_native_refusals={count}"
    );
}
