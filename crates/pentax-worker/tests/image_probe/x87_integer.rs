// SPDX-License-Identifier: GPL-2.0-only
//! Original checked x87 signed-integer conversion consumers. Independent integer
//! literals and guest branches consume result bytes, exact widths and state;
//! no host floating-point conversion oracle is used.
use super::*;

const ENTRY: i32 = 0x1000;
const CHILD: i32 = 0x5000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const INDEFINITE: (u64, u16) = (0xc000_0000_0000_0000, 0xffff);
const CONTROL: u16 = 0x037f;
const MXCSR: u32 = 0x5f84;
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
            bytes: vec![0x55, 0x48, 0x81, 0xec, 0, 1, 0, 0],
            failed: Vec::new(),
        }
    }
    fn reg(&mut self, register: u8, value: u64) {
        self.bytes
            .extend([0x48 | u8::from(register >= 8), 0xb8 + (register & 7)]);
        self.bytes.extend(value.to_le_bytes());
    }
    fn stack(&mut self, opcode: u8, register: u8, offset: u8) {
        self.bytes.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            opcode,
            (if offset < 128 { 0x44 } else { 0x84 }) | ((register & 7) << 3),
            0x24,
        ]);
        if offset < 128 {
            self.bytes.push(offset);
        } else {
            self.bytes.extend(u32::from(offset).to_le_bytes());
        }
    }
    fn word(&mut self, offset: u8, value: u64) {
        self.reg(0, value);
        self.stack(0x89, 0, offset);
    }
    fn bad_if(&mut self, condition: u8) {
        self.bytes.extend([0x0f, condition, 0, 0, 0, 0]);
        self.failed.push(self.bytes.len() - 4);
    }
    fn bad(&mut self) {
        self.bad_if(0x85);
    }
    fn expect(&mut self, register: u8, value: u64) {
        self.reg(10, value);
        self.bytes
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        self.bad();
    }
    fn check(&mut self, offset: u8, value: u64) {
        self.stack(0x8b, 0, offset);
        self.expect(0, value);
    }
    fn call(&mut self, slot: i32) {
        rip(&mut self.bytes, self.base, &[0xff, 0x15], 0xf00 + slot * 8);
    }
    fn flags(&mut self) {
        self.reg(0, CANARY);
        self.reg(10, gpr_pattern(10));
        self.bytes.extend([0x48, 0x39, 0xc0, 0xf9]); // CMP RAX,RAX; STC.
    }
    fn check_flags(&mut self) {
        // CMP-equal plus STC set CF/PF/ZF and clear SF/OF. These admitted
        // conditional branches inspect them without touching GPRs or flags.
        // AF is covered by the separate full-state platform fixtures.
        for condition in [0x83, 0x85, 0x8b, 0x88, 0x80] {
            self.bad_if(condition);
        }
    }
    fn begin(&mut self, bits: (u64, u16)) {
        self.bytes.extend([0xdb, 0xe3]); // FNINIT.
        self.word(96, u64::from(CONTROL));
        self.bytes.extend([0xd9, 0x6c, 0x24, 96]); // FLDCW.
        self.word(16, bits.0);
        self.word(24, u64::from(bits.1));
        self.flags();
    }
    fn pop80(&mut self, value: (u64, u16), status: u16) {
        self.stack(0x89, 0, 160);
        self.word(48, CANARY);
        self.word(56, CANARY);
        self.bytes.extend([0xdb, 0x7c, 0x24, 48]); // FSTP m80fp: exactly 10 bytes.
        self.stack(0x89, 0, 168);
        self.stack(0x89, 10, 144);
        self.check_flags();
        self.check(160, CANARY);
        self.check(168, CANARY);
        self.check(144, gpr_pattern(10));
        self.check(48, value.0);
        self.check(56, (CANARY & !0xffff) | u64::from(value.1));
        self.word(64, CANARY);
        self.bytes.extend([0xdd, 0x7c, 0x24, 64]); // FNSTSW m16.
        self.check(64, (CANARY & !0xffff) | u64::from(status));
        self.word(72, CANARY);
        self.bytes.extend([0xd9, 0x7c, 0x24, 72]); // FNSTCW m16.
        self.check(72, (CANARY & !0xffff) | u64::from(CONTROL));
    }
    fn vector(&mut self, load: bool, register: u8) {
        self.bytes.extend([
            0xc5,
            if register >= 8 { 0x7c } else { 0xfc },
            if load { 0x10 } else { 0x11 },
            0x44 | ((register & 7) << 3),
            0x24,
            112,
        ]);
    }
    fn seed_context(&mut self) {
        self.stack(0x89, 4, 0); // Actual current RSP, with the owned frame retained.
        self.stack(0x89, 5, 152);
        self.word(8, u64::from(MXCSR));
        self.bytes.extend([0x0f, 0xae, 0x54, 0x24, 8]);
        for register in 0..16 {
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.word(112 + i as u8 * 8, value);
            }
            self.vector(true, register);
        }
        for register in GPRS {
            self.reg(register, gpr_pattern(register));
        }
    }
    fn check_context(&mut self) {
        for register in GPRS {
            self.expect(register, gpr_pattern(register));
        }
        for register in 0..16 {
            self.vector(false, register);
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.check(112 + i as u8 * 8, value);
            }
        }
        self.stack(0x8b, 0, 0);
        self.bytes.extend([0x48, 0x39, 0xc4]); // CMP RSP,RAX.
        self.bad();
        self.stack(0x8b, 0, 152);
        self.bytes.extend([0x48, 0x39, 0xc5]); // CMP RBP,RAX.
        self.bad();
        self.word(88, CANARY);
        self.bytes.extend([0x0f, 0xae, 0x5c, 0x24, 88]);
        self.check(88, (CANARY & !0xffff_ffff) | u64::from(MXCSR));
    }
    fn finish(mut self, value: u64) -> Vec<u8> {
        self.reg(0, value);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x4000);
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

// Independently bounded system-import image: immutable RX text/metadata only;
// all mutable operands and outputs live in the worker's actual owned stacks.
fn image(main: &[u8], child: Option<&[u8]>) -> Input {
    let roles = [
        pentax_kernel::Service::PthreadCreate,
        pentax_kernel::Service::PthreadJoin,
        pentax_kernel::Service::Getpid,
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
    assert!(main.len() < (CHILD - ENTRY) as usize);
    put(&mut bytes, ENTRY as usize, main);
    if let Some(child) = child {
        assert!(child.len() < 0x4000);
        put(&mut bytes, CHILD as usize, child);
    }
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
        (39, 4 * 24),
        (11, 24),
        (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
        (0x6100_0015, 11 | (1 << 32) | (3 << 48)),
        (7, 0x700),
        (8, 3 * 24),
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
fn run(source: &Input) -> (i32, HashMap<String, String>, String) {
    probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "extended-startup".into(),
    ])
}

const ZERO: (u64, u16) = (0, 0);
const THREE: (u64, u16) = (0xc000_0000_0000_0000, 0x4000);
const ONE_HALF: (u64, u16) = (0xc000_0000_0000_0000, 0x3fff);
const TWO_HALF: (u64, u16) = (0xa000_0000_0000_0000, 0x4000);
const THREE_HALF: (u64, u16) = (0xe000_0000_0000_0000, 0x4000);
const INF: (u64, u16) = (0x8000_0000_0000_0000, 0x7fff);
const QUIET: (u64, u16) = (0xc000_0000_0000_0055, 0x7fff);
#[derive(Clone, Copy)]
struct Store {
    opcode: u8,
    operand: u8,
    width: u8,
    pops: bool,
    truncates: bool,
}
const STORES: [Store; 8] = [
    Store {
        opcode: 0xdf,
        operand: 0x54,
        width: 16,
        pops: false,
        truncates: false,
    },
    Store {
        opcode: 0xdb,
        operand: 0x54,
        width: 32,
        pops: false,
        truncates: false,
    },
    Store {
        opcode: 0xdf,
        operand: 0x5c,
        width: 16,
        pops: true,
        truncates: false,
    },
    Store {
        opcode: 0xdb,
        operand: 0x5c,
        width: 32,
        pops: true,
        truncates: false,
    },
    Store {
        opcode: 0xdf,
        operand: 0x7c,
        width: 64,
        pops: true,
        truncates: false,
    },
    Store {
        opcode: 0xdf,
        operand: 0x4c,
        width: 16,
        pops: true,
        truncates: true,
    },
    Store {
        opcode: 0xdb,
        operand: 0x4c,
        width: 32,
        pops: true,
        truncates: true,
    },
    Store {
        opcode: 0xdd,
        operand: 0x4c,
        width: 64,
        pops: true,
        truncates: true,
    },
];
fn integer_words(width: u8, bits: u64) -> (u64, u64) {
    match width {
        16 => (
            (CANARY & !(0xffff_u64 << 8)) | ((bits & 0xffff) << 8),
            CANARY,
        ),
        32 => (
            (CANARY & !(0xffff_ffff_u64 << 8)) | ((bits & 0xffff_ffff) << 8),
            CANARY,
        ),
        64 => (
            (CANARY & 0xff) | (bits << 8),
            (CANARY & !0xff) | (bits >> 56),
        ),
        _ => unreachable!(),
    }
}
fn minimum(width: u8) -> i64 {
    match width {
        16 => -32768,
        32 => -2147483648,
        64 => i64::MIN,
        _ => unreachable!(),
    }
}
impl Code {
    fn push80(&mut self, value: (u64, u16)) {
        self.word(16, value.0);
        self.word(24, u64::from(value.1));
        self.bytes.extend([0xdb, 0x6c, 0x24, 16]);
    }
    fn control(&mut self, control: u16) {
        self.word(96, u64::from(control));
        self.bytes.extend([0xd9, 0x6c, 0x24, 96]);
    }
    fn data_flags(&mut self) {
        self.stack(0x89, 0, 176);
        self.stack(0x89, 10, 184);
        self.check_flags();
        self.check(176, CANARY);
        self.check(184, gpr_pattern(10));
    }
    fn status(&mut self, expected: u16, control: u16) {
        self.word(64, CANARY);
        self.bytes.extend([0xdd, 0x7c, 0x24, 64]);
        self.check(64, (CANARY & !0xffff) | u64::from(expected));
        self.word(72, CANARY);
        self.bytes.extend([0xd9, 0x7c, 0x24, 72]);
        self.check(72, (CANARY & !0xffff) | u64::from(control));
    }
    fn store_result(&mut self, form: Store, result: i64, status: u16, control: u16) {
        self.word(48, CANARY);
        self.word(56, CANARY);
        self.flags();
        self.bytes.extend([form.opcode, form.operand, 0x24, 49]); // Unaligned exact-width destination.
        self.data_flags();
        self.status(status, control);
        let (first, last) = integer_words(form.width, result as u64);
        self.check(48, first);
        self.check(56, last);
    }
    fn store_case(
        &mut self,
        form: Store,
        value: (u64, u16),
        control: u16,
        result: i64,
        status: u16,
    ) {
        self.begin(ZERO);
        self.push80(value);
        self.control(control);
        self.store_result(
            form,
            result,
            (if form.pops { 0 } else { 0x3800 }) | status,
            control,
        );
        self.control(CONTROL);
        self.flags();
        if form.pops {
            self.pop80(INDEFINITE, 0x0841 | (status & 0x7f));
        } else {
            self.pop80(value, status & 0x7f);
        }
    }
    fn empty_store(&mut self, form: Store) {
        self.begin(ZERO);
        self.store_result(
            form,
            minimum(form.width),
            (if form.pops { 0x0800 } else { 0 }) | 0x41,
            CONTROL,
        );
        self.flags();
        self.pop80(INDEFINITE, (if form.pops { 0x1000 } else { 0x0800 }) | 0x41);
    }
}
fn first_fisttp() -> Input {
    let mut p = Code::new(ENTRY);
    // RC=ceil would produce4 through FISTP; FISTTP must truncate3.75 to3.
    p.store_case(STORES[7], (0xf000_0000_0000_0000, 0x4000), 0x0b7f, 3, 0x20);
    image(&p.finish(42), None)
}
fn width_matrix(width: u8) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for form in STORES.into_iter().filter(|form| form.width == width) {
        p.store_case(form, (THREE.0, 0xc000), CONTROL, -3, 0);
        p.store_case(
            form,
            ONE_HALF,
            0x0b7f,
            if form.truncates { 1 } else { 2 },
            if form.truncates { 0x20 } else { 0x220 },
        );
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn rounding_matrix(negative: bool) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    let sign = if negative { 0x8000 } else { 0 };
    for rc in 0..4_u16 {
        let magnitude_increment = (!negative && rc == 2) || (negative && rc == 1);
        let result = match (negative, rc) {
            (false, 2) => 3,
            (false, _) => 2,
            (true, 1) => -3,
            (true, _) => -2,
        };
        p.store_case(
            STORES[4],
            (TWO_HALF.0, TWO_HALF.1 | sign),
            0x037f | (rc << 10),
            result,
            0x20 | if magnitude_increment { 0x200 } else { 0 },
        );
    }
    p.store_case(
        STORES[4],
        (THREE_HALF.0, THREE_HALF.1 | sign),
        CONTROL,
        if negative { -4 } else { 4 },
        0x220,
    );
    // PC24 cannot first round this full80 value to the exact2.5 tie.
    if !negative {
        p.store_case(STORES[4], (TWO_HALF.0 + 1, TWO_HALF.1), 0x007f, 3, 0x220);
    }
    // A true80 subnormal does not raise DE for integer stores, even with DM=0.
    p.store_case(STORES[4], (1, sign), 0x037d, 0, 0x20);
    p.check_context();
    image(&p.finish(42), None)
}
fn truncation_matrix() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for negative in [false, true] {
        let sign = if negative { 0x8000 } else { 0 };
        for rc in 0..4_u16 {
            p.store_case(
                STORES[7],
                (TWO_HALF.0, TWO_HALF.1 | sign),
                0x037f | (rc << 10),
                if negative { -2 } else { 2 },
                0x20,
            );
        }
        p.store_case(STORES[7], (1, sign), 0x037d, 0, 0x20);
    }
    p.store_case(STORES[7], (TWO_HALF.0 + 1, TWO_HALF.1), 0x007f, 2, 0x20);
    p.check_context();
    image(&p.finish(42), None)
}
fn load_integer_matrix() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    let cases = [
        (16_u8, -32768_i64, (0x8000_0000_0000_0000, 0xc00e), false),
        (16, 32767, (0xfffe_0000_0000_0000, 0x400d), false),
        (32, -2147483648, (0x8000_0000_0000_0000, 0xc01e), false),
        (32, 2147483647, (0xffff_fffe_0000_0000, 0x401d), false),
        (64, i64::MIN, (0x8000_0000_0000_0000, 0xc03e), false),
        (64, i64::MAX, (0xffff_ffff_ffff_fffe, 0x403d), false),
        (64, 9007199254740993, (0x8000_0000_0000_0400, 0x4034), false),
        (64, i64::MAX, (0xffff_ffff_ffff_fffe, 0xc03d), true),
    ];
    for (width, value, extended, negate) in cases {
        p.begin(ZERO);
        p.control(0x007f);
        let (first, last) = integer_words(width, value as u64);
        p.word(16, first);
        p.word(24, last);
        p.flags();
        p.bytes.extend([
            if width == 32 { 0xdb } else { 0xdf },
            if width == 64 { 0x6c } else { 0x44 },
            0x24,
            17,
        ]);
        p.data_flags();
        p.status(0x3800, 0x007f);
        p.check(16, first);
        p.check(24, last);
        if negate {
            // Existing FCHS/FABS data forms are exercised, not claimed as new.
            for opcode in [0xe0, 0xe1, 0xe0] {
                p.flags();
                p.bytes.extend([0xd9, opcode]);
                p.data_flags();
                p.status(0x3800, 0x007f);
            }
        }
        p.flags();
        p.bytes.extend([0xd9, 0xc0]);
        p.data_flags();
        p.status(0x3000, 0x007f); // Duplicate actual full80 value.
        let form = match width {
            16 => STORES[2],
            32 => STORES[3],
            64 => STORES[4],
            _ => unreachable!(),
        };
        p.store_result(form, if negate { -value } else { value }, 0x3800, 0x007f);
        p.control(CONTROL);
        p.flags();
        p.pop80(extended, 0);
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn invalid_and_empty_matrix() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for (index, form) in STORES.into_iter().enumerate() {
        let exponent = 0x3fff + u16::from(form.width) - 1;
        let value = match index % 4 {
            0 => QUIET,
            1 => INF,
            2 => (0x8000_0000_0000_0000, exponent),
            _ => (
                0x8000_0000_0000_0000 | (1_u64 << (64 - form.width)),
                exponent | 0x8000,
            ),
        };
        p.store_case(form, value, CONTROL, minimum(form.width), 1);
    }
    p.empty_store(STORES[1]);
    p.empty_store(STORES[7]);
    p.check_context();
    image(&p.finish(42), None)
}
fn boundary_rounding_matrix() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    let form = STORES[2];
    let high_half = (0xffff_0000_0000_0000, 0x400d); //32767.5.
    p.store_case(form, high_half, CONTROL, -32768, 1); // Rounded32768 is invalid, suppressing PE.
    p.store_case(form, high_half, 0x0f7f, 32767, 0x20);
    let low_fraction = (0x8000_c000_0000_0000, 0xc00e); //-32768.75.
    p.store_case(form, low_fraction, 0x0f7f, -32768, 0x20);
    p.store_case(form, low_fraction, 0x077f, -32768, 1); // Floor is outside signed16.
    p.store_case(form, (0x8000_8000_0000_0000, 0xc00e), CONTROL, -32768, 0x20); // Even minimum is valid.
    p.store_case(form, (0x8000_0000_0000_0000, 0xc00e), CONTROL, -32768, 0); // Same bits, no invalid.
    p.check_context();
    image(&p.finish(42), None)
}
fn threaded_integer() -> Input {
    let mut main = Code::new(ENTRY);
    main.begin(ZERO);
    main.push80((0x8000_0000_0000_0003, 0x3fff));
    main.stack(0x8d, 7, 0);
    main.reg(6, 0);
    rip(&mut main.bytes, main.base, &[0x48, 0x8d, 0x15], CHILD);
    main.reg(1, 0);
    main.reg(8, 0);
    main.call(0);
    main.expect(0, 0);
    main.stack(0x8b, 7, 0);
    main.stack(0x8d, 6, 32);
    main.call(1);
    main.expect(0, 0);
    main.check(32, 77);
    main.flags();
    main.pop80((0x8000_0000_0000_0003, 0x3fff), 0);
    let mut child = Code::new(CHILD);
    child.seed_context();
    child.begin(ZERO);
    child.push80(ONE_HALF);
    child.control(0x0b7f);
    child.store_result(STORES[1], 2, 0x3a20, 0x0b7f); // Nonpop ceil, real retained operand and C1.
    child.call(2);
    child.expect(0, 1);
    child.reg(1, gpr_pattern(1));
    child.reg(11, gpr_pattern(11));
    child.status(0x3a20, 0x0b7f);
    child.store_result(STORES[6], 1, 0x20, 0x0b7f); // TP ignores retained ceil and clears set C1.
    child.control(CONTROL);
    child.flags();
    child.pop80(INDEFINITE, 0x0861);
    child.check_context();
    image(&main.finish(42), Some(&child.finish(77)))
}
fn refused(status: i32, fields: &HashMap<String, String>, text: &str) {
    assert_ne!(status, 0, "{text}");
    assert_eq!(fields["code"], "native-dispatch");
    assert_eq!(fields["runtime_failure"], "x87-exception");
    assert!(!fields.contains_key("value"));
    assert_eq!(fields["service_stops"], "0");
}
#[test]
fn native_x87_integer_stores_loads_rounding_exact_widths_and_current_thread_context_have_integer_consumers()
 {
    let first = first_fisttp();
    let (status, fields, text) = run(&first);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
    let programs = [
        width_matrix(16),
        width_matrix(32),
        width_matrix(64),
        rounding_matrix(false),
        rounding_matrix(true),
        truncation_matrix(),
        load_integer_matrix(),
        invalid_and_empty_matrix(),
        boundary_rounding_matrix(),
        threaded_integer(),
    ];
    let mut completed = 1;
    for (index, source) in programs.iter().enumerate() {
        let (status, fields, text) = run(source);
        assert_eq!(status, 0, "program={index}: {text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["code"], "outer-return");
        assert_eq!(fields["fp_status"], if index == 9 { "0" } else { "4" });
        assert_eq!(fields["service_stops"], if index == 9 { "3" } else { "0" });
        if index == 9 {
            assert_eq!(fields["scheduling_threads"], "2");
            assert_eq!(fields["scheduling_returned"], "1");
        }
        completed += 1;
    }
    assert_eq!(completed, 11);
    println!(
        "x87_integer_native_available=true x87_integer_native_programs={completed} x87_integer_native_returns={completed}"
    );
}
#[test]
fn native_x87_integer_new_unmasked_and_pending_exceptions_refuse_without_return() {
    let mut completed = 0;
    for kind in 0..6 {
        let mut p = Code::new(ENTRY);
        p.begin(ZERO);
        match kind {
            0 => {
                p.push80(ONE_HALF);
                p.control(0x035f);
                p.word(48, CANARY);
                p.word(56, CANARY);
                p.flags();
                p.bytes.extend([0xdf, 0x7c, 0x24, 49]);
            }
            1 => {
                p.push80(INF);
                p.control(0x037e);
                p.word(48, CANARY);
                p.word(56, CANARY);
                p.flags();
                p.bytes.extend([0xdf, 0x54, 0x24, 49]);
            }
            2 => {
                p.control(0x037e);
                p.word(48, CANARY);
                p.word(56, CANARY);
                p.flags();
                p.bytes.extend([0xdb, 0x4c, 0x24, 49]);
            }
            // Exact FILD3 would not raise IE/PE: these stops genuinely require
            // the pending exception, rather than a second faulting conversion.
            3 => {
                p.push80(INF);
                p.store_result(STORES[2], -32768, 1, CONTROL);
                p.word(16, 3);
                p.control(0x037e);
                p.flags();
                p.bytes.extend([0xdf, 0x44, 0x24, 16]);
            }
            4 => {
                p.push80(ONE_HALF);
                p.store_result(STORES[1], 2, 0x3a20, CONTROL);
                p.word(16, 3);
                p.control(0x035f);
                p.flags();
                p.bytes.extend([0xdf, 0x44, 0x24, 16]);
            }
            _ => {
                p.control(0x037e);
                for _ in 0..8 {
                    p.bytes.extend([0xd9, 0xe8]);
                }
                p.word(16, 3);
                p.flags();
                p.bytes.extend([0xdf, 0x44, 0x24, 16]);
            }
        }
        let source = image(&p.finish(42), None);
        let (status, fields, text) = run(&source);
        refused(status, &fields, &text);
        completed += 1;
    }
    assert_eq!(completed, 6);
    println!(
        "x87_integer_refusal_native_available=true x87_integer_refusal_native_programs={completed} x87_integer_refusal_native_returns=0"
    );
}
