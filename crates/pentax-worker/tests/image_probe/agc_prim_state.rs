// SPDX-License-Identifier: GPL-2.0-only
//! Original exact-output consumers of primitive-state construction.
//! Constructed register pairs do not submit commands or establish a rendered frame.
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
            if matches!(register, 1 | 11 | 7 | 6 | 2 | 8) {
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
    for (index, nid) in ["D9sr1xGUriE", "t7PlZ9nt5Lc", "9BcDykPmo1I"]
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
const GS: u32 = 256;
const HS: u32 = 384;
const GS_SPECIALS: u32 = 512;
const HS_SPECIALS: u32 = 576;
const CX: u32 = 768;
const UC: u32 = 800;
const GS_OUT: u32 = 0xdead_beef;
const HS_OUT: u32 = 0x1122_3344;
const GS_GE: u32 = 0x8765_4321;
const GS_USER: u32 = 0x89ab_cdef;
const HS_USER: u32 = 0xfedc_ba98;
const PRIMITIVES: [u32; 20] = [
    0,
    1,
    2,
    3,
    4,
    5,
    6,
    7,
    8,
    9,
    10,
    11,
    12,
    13,
    17,
    18,
    19,
    20,
    21,
    0xffff_ffff,
];
#[derive(Clone, Copy)]
struct Configuration {
    gs_geometry: bool,
    hs_geometry: Option<bool>,
    cx: bool,
    uc: bool,
}
fn pair(offset: u32, value: u32) -> u64 {
    u64::from(offset) | (u64::from(value) << 32)
}
fn primitive_output(primitive: u32) -> u32 {
    match primitive {
        1 => 0,
        2 | 3 | 10 | 11 | 18 => 1,
        7 => 3,
        17 => 4,
        _ => 2,
    }
}
fn stages(geometry: bool, hull: bool) -> u32 {
    (if hull { 0x100 } else { 0x40 }) | if geometry { 0x20 } else { 0 }
}
fn initialize(p: &mut Code, configuration: Configuration) {
    for (shader, specials, kind) in [(GS, GS_SPECIALS, 2_u8), (HS, HS_SPECIALS, 3)] {
        for offset in (0..96).step_by(8) {
            p.word(shader + offset, CANARY);
        }
        p.pointer_field(shader + 40, specials);
        p.word(
            shader + 88,
            (CANARY & !(0xff << 16)) | (u64::from(kind) << 16),
        );
        for offset in (0..48).step_by(8) {
            p.word(specials + offset, CANARY);
        }
    }
    p.word(
        GS_SPECIALS,
        pair(if configuration.uc { 0x25b } else { 0xbad }, GS_GE),
    );
    p.word(
        GS_SPECIALS + 8,
        pair(
            if configuration.cx { 0x2d5 } else { 0xbad },
            stages(configuration.gs_geometry, false),
        ),
    );
    p.word(
        GS_SPECIALS + 32,
        pair(if configuration.cx { 0x29b } else { 0xbad }, GS_OUT),
    );
    p.word(
        GS_SPECIALS + 40,
        pair(if configuration.uc { 0x262 } else { 0xbad }, GS_USER),
    );
    // HS.GE_CNTL is deliberately unrelated: this API never consumes it.
    p.word(HS_SPECIALS, pair(0xbad, 0x1357_9bdf));
    p.word(
        HS_SPECIALS + 8,
        pair(
            if configuration.cx { 0x2d5 } else { 0xbad },
            stages(configuration.hs_geometry.unwrap_or(false), true),
        ),
    );
    p.word(
        HS_SPECIALS + 32,
        pair(if configuration.cx { 0x29b } else { 0xbad }, HS_OUT),
    );
    p.word(
        HS_SPECIALS + 40,
        pair(if configuration.uc { 0x262 } else { 0xbad }, HS_USER),
    );
    p.call(2);
    p.stack(0x89, 0, 64);
    p.bytes.extend([0xc7, 0, 0x37, 0, 0, 0]);
}
fn output_canaries(p: &mut Code) {
    for offset in [
        CX - 8,
        CX,
        CX + 8,
        CX + 16,
        UC - 8,
        UC,
        UC + 8,
        UC + 16,
        UC + 24,
    ] {
        p.word(offset, CANARY);
    }
}
fn arguments(p: &mut Code, configuration: Configuration, primitive: u64) {
    if configuration.cx {
        p.ptr(7, CX);
    } else {
        p.reg(7, 0);
    }
    if configuration.uc {
        p.ptr(6, UC);
    } else {
        p.reg(6, 0);
    }
    if configuration.hs_geometry.is_some() {
        p.ptr(2, HS);
    } else {
        p.reg(2, 0);
    }
    p.ptr(1, GS);
    p.reg(8, primitive);
}
fn check_arguments(p: &mut Code, configuration: Configuration, primitive: u64) {
    if configuration.cx {
        p.same_ptr(7, CX);
    } else {
        p.expect(7, 0);
    }
    if configuration.uc {
        p.same_ptr(6, UC);
    } else {
        p.expect(6, 0);
    }
    if configuration.hs_geometry.is_some() {
        p.same_ptr(2, HS);
    } else {
        p.expect(2, 0);
    }
    p.expect(8, primitive); // RCX is the existing syscall clobber, not a retained GPR.
}
fn check_outputs(p: &mut Code, configuration: Configuration, primitive: u32) {
    for offset in [CX - 8, CX + 16, UC - 8, UC + 24] {
        p.check(offset, CANARY);
    }
    if configuration.cx {
        let gs = stages(configuration.gs_geometry, false);
        let hs = configuration
            .hs_geometry
            .map_or(0, |geometry| stages(geometry, true));
        let output = if gs & 0x20 != 0 {
            GS_OUT
        } else if configuration.hs_geometry.is_some() && (gs | hs) & 0x20 == 0 {
            HS_OUT
        } else {
            primitive_output(primitive)
        };
        p.check(CX, pair(0x2d5, gs | hs));
        p.check(CX + 8, pair(0x29b, output));
    } else {
        p.check(CX, CANARY);
        p.check(CX + 8, CANARY);
    }
    if configuration.uc {
        p.check(UC, pair(0x25b, GS_GE));
        p.check(
            UC + 8,
            pair(
                0x262,
                if configuration.hs_geometry.is_some() {
                    HS_USER
                } else {
                    GS_USER
                },
            ),
        );
        p.check(UC + 16, pair(0x242, primitive));
    } else {
        for offset in [UC, UC + 8, UC + 16] {
            p.check(offset, CANARY);
        }
    }
}
fn check_sources(p: &mut Code, configuration: Configuration) {
    for (shader, specials, kind) in [(GS, GS_SPECIALS, 2_u8), (HS, HS_SPECIALS, 3)] {
        for offset in (0..96).step_by(8) {
            if offset == 40 {
                p.field_is_pointer(shader + offset, specials);
            } else {
                p.check(
                    shader + offset,
                    if offset == 88 {
                        (CANARY & !(0xff << 16)) | (u64::from(kind) << 16)
                    } else {
                        CANARY
                    },
                );
            }
        }
    }
    for specials in [GS_SPECIALS, HS_SPECIALS] {
        p.check(specials + 16, CANARY);
        p.check(specials + 24, CANARY);
    }
    p.check(
        GS_SPECIALS,
        pair(if configuration.uc { 0x25b } else { 0xbad }, GS_GE),
    );
    p.check(
        GS_SPECIALS + 8,
        pair(
            if configuration.cx { 0x2d5 } else { 0xbad },
            stages(configuration.gs_geometry, false),
        ),
    );
    p.check(
        GS_SPECIALS + 32,
        pair(if configuration.cx { 0x29b } else { 0xbad }, GS_OUT),
    );
    p.check(
        GS_SPECIALS + 40,
        pair(if configuration.uc { 0x262 } else { 0xbad }, GS_USER),
    );
    p.check(HS_SPECIALS, pair(0xbad, 0x1357_9bdf));
    p.check(
        HS_SPECIALS + 8,
        pair(
            if configuration.cx { 0x2d5 } else { 0xbad },
            stages(configuration.hs_geometry.unwrap_or(false), true),
        ),
    );
    p.check(
        HS_SPECIALS + 32,
        pair(if configuration.cx { 0x29b } else { 0xbad }, HS_OUT),
    );
    p.check(
        HS_SPECIALS + 40,
        pair(if configuration.uc { 0x262 } else { 0xbad }, HS_USER),
    );
}
fn check_tls(p: &mut Code) {
    p.call(2);
    p.stack(0x8b, 10, 64);
    p.bytes.extend([0x4c, 0x39, 0xd0]);
    p.bad();
    p.bytes.extend([0x83, 0x38, 0x37]);
    p.bad();
}
fn matrix(scope: Scope, configuration: Configuration, primitives: &[u32]) -> Input {
    let mut p = Code::new(ENTRY);
    initialize(&mut p, configuration);
    p.seed_context();
    p.hold_x87();
    for &primitive in primitives {
        output_canaries(&mut p);
        let argument = 0x7654_3210_0000_0000 | u64::from(primitive);
        arguments(&mut p, configuration, argument);
        p.flags();
        p.call(0);
        p.check_flags();
        p.expect(0, 0);
        check_arguments(&mut p, configuration, argument);
        check_outputs(&mut p, configuration, primitive);
    }
    check_sources(&mut p, configuration);
    check_tls(&mut p);
    p.check_context(ENTRY);
    p.check_x87();
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}

fn aliased(scope: Scope, case: u8) -> Input {
    let configuration = Configuration {
        gs_geometry: case == 0,
        hs_geometry: if case == 1 { Some(false) } else { None },
        cx: case != 2,
        uc: case == 2,
    };
    let mut p = Code::new(ENTRY);
    initialize(&mut p, configuration);
    output_canaries(&mut p);
    p.seed_context();
    p.hold_x87();
    let primitive = 0x1234_5678_ffff_ffff;
    arguments(&mut p, configuration, primitive);
    let destination = match case {
        0 => GS_SPECIALS + 32,
        1 => HS_SPECIALS + 8,
        2 => GS_SPECIALS + 40,
        _ => unreachable!("original alias case"),
    };
    p.ptr(if case == 2 { 6 } else { 7 }, destination);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    p.same_ptr(if case == 2 { 6 } else { 7 }, destination);
    p.expect(if case == 2 { 7 } else { 6 }, 0);
    if case == 1 {
        p.same_ptr(2, HS);
    } else {
        p.expect(2, 0);
    }
    p.expect(8, primitive);
    match case {
        0 => {
            // First write overwrites GS.out; the next copy must read that current pair.
            p.check(GS_SPECIALS + 32, pair(0x2d5, 0x60));
            p.check(GS_SPECIALS + 40, pair(0x2d5, 0x60));
            p.check(GS_SPECIALS + 8, pair(0x2d5, 0x60));
        }
        1 => {
            // CX0 overwrites HS.stages before OR, so the original HS0x100 is absent.
            p.check(HS_SPECIALS + 8, pair(0x2d5, 0x40));
            p.check(HS_SPECIALS + 16, pair(0x29b, HS_OUT));
            p.check(HS_SPECIALS + 32, pair(0x29b, HS_OUT));
        }
        2 => {
            // UC0 overwrites GS.user before UC1 reads it; UC2 retains all32 prim bits.
            p.check(GS_SPECIALS + 40, pair(0x25b, GS_GE));
            p.check(GS_SPECIALS + 48, pair(0x25b, GS_GE));
            p.check(GS_SPECIALS + 56, pair(0x242, u32::MAX));
            p.check(HS_SPECIALS, pair(0xbad, 0x1357_9bdf));
        }
        _ => unreachable!("original alias case"),
    }
    for offset in [
        CX - 8,
        CX,
        CX + 8,
        CX + 16,
        UC - 8,
        UC,
        UC + 8,
        UC + 16,
        UC + 24,
    ] {
        p.check(offset, CANARY);
    }
    p.field_is_pointer(GS + 40, GS_SPECIALS);
    p.field_is_pointer(HS + 40, HS_SPECIALS);
    check_tls(&mut p);
    p.check_context(ENTRY);
    p.check_x87();
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
fn both_null(scope: Scope) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.hold_x87();
    p.reg(7, 0);
    p.reg(6, 0);
    p.reg(2, u64::MAX);
    p.reg(1, u64::MAX);
    p.reg(8, u64::MAX);
    p.flags();
    p.call(0);
    p.check_flags();
    p.expect(0, 0);
    p.expect(7, 0);
    p.expect(6, 0);
    p.expect(2, u64::MAX);
    p.expect(8, u64::MAX);
    p.check_context(ENTRY);
    p.check_x87();
    image(&p.finish(42), &[0x0f, 0x0b], scope)
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
fn native_agc_create_prim_state_consumes_exact_pairs_branches_aliases_and_full_context() {
    let mut count = 0;
    for scope in [Scope::Canonical, Scope::Observed] {
        for gs_geometry in [false, true] {
            for hs_geometry in [None, Some(false), Some(true)] {
                let configuration = Configuration {
                    gs_geometry,
                    hs_geometry,
                    cx: true,
                    uc: true,
                };
                for primitives in PRIMITIVES.chunks(10) {
                    returned(
                        &matrix(scope, configuration, primitives),
                        primitives.len() as u64 + 2,
                        primitives.len() as u64,
                    );
                    count += 1;
                }
            }
        }
        for (cx, uc) in [(true, false), (false, true)] {
            let configuration = Configuration {
                gs_geometry: false,
                hs_geometry: Some(true),
                cx,
                uc,
            };
            for primitives in PRIMITIVES.chunks(10) {
                returned(
                    &matrix(scope, configuration, primitives),
                    primitives.len() as u64 + 2,
                    primitives.len() as u64,
                );
                count += 1;
            }
        }
        for case in 0..3 {
            returned(&aliased(scope, case), 3, 1);
            count += 1;
        }
        returned(&both_null(scope), 1, 1);
        count += 1;
    }
    assert_eq!(count, 40);
    println!(
        "agc_prim_state_native_available=true agc_prim_state_native_programs={count} agc_prim_state_native_returns={count}"
    );
}
fn failure(scope: Scope, case: u8) -> Input {
    let configuration = Configuration {
        gs_geometry: false,
        hs_geometry: Some(false),
        cx: true,
        uc: true,
    };
    let mut p = Code::new(ENTRY);
    initialize(&mut p, configuration);
    output_canaries(&mut p);
    arguments(&mut p, configuration, 4);
    match case {
        0 => p.reg(1, 0),
        1 => p.word(GS + 88, CANARY & !(0xff << 16)),
        2 => p.word(HS + 88, CANARY & !(0xff << 16)),
        3 => p.word(GS + 40, u64::MAX),
        4 => p.word(HS + 40, u64::MAX),
        5 => p.reg(1, u64::MAX),
        6 => p.reg(2, u64::MAX),
        7 => p.word(GS_SPECIALS + 8, pair(0xbad, 0x40)),
        8 => p.word(GS_SPECIALS + 32, pair(0xbad, GS_OUT)),
        9 => p.word(HS_SPECIALS + 8, pair(0xbad, 0x100)),
        10 => p.word(HS_SPECIALS + 32, pair(0xbad, HS_OUT)),
        11 => p.word(GS_SPECIALS, pair(0xbad, GS_GE)),
        12 => p.word(GS_SPECIALS + 40, pair(0xbad, GS_USER)),
        13 => p.word(HS_SPECIALS + 40, pair(0xbad, HS_USER)),
        14 => p.path(7, ENTRY),
        15 => p.path(6, ENTRY),
        _ => unreachable!("original typed refusal"),
    }
    p.call(0);
    image(&p.finish(42), &[0x0f, 0x0b], scope)
}
#[test]
fn native_agc_create_prim_state_refuses_shader_memory_register_and_exact_scope_failures() {
    let mut count = 0;
    for case in 0..16 {
        let source = failure(Scope::Observed, case);
        let (status, fields, text) = run(&source);
        let expected = match case {
            0 => "shader-null-argument",
            1 | 2 => "shader-stage",
            7..=13 => "shader-registers",
            _ => "graphics-memory",
        };
        assert_eq!(status, 2, "case={case}: {text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], expected, "case={case}: {text}");
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
    ] {
        let source = failure(scope, 0);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "scope={scope:?}: {text}");
        if matches!(scope, Scope::WrongPair) {
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
    assert_eq!(count, 19);
    println!(
        "agc_prim_state_native_refusal_programs={count} agc_prim_state_native_refusals={count}"
    );
}
