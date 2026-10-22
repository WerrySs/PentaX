// SPDX-License-Identifier: GPL-2.0-only
//! Original integer consumers for all eight checked x87 conditional moves.
//! Exact stored80 results, source retention and flags determine the guest return;
//! no host floating-point oracle or imported guest program is used.
use super::*;

const ENTRY: i32 = 0x1000;
const CHILD: i32 = 0x5000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const ONE: (u64, u16) = (0x8000_0000_0000_0000, 0x3fff);
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
const QUIET: (u64, u16) = (0xc000_0000_0000_0055, 0x7fff);
const INF: (u64, u16) = (0x8000_0000_0000_0000, 0x7fff);
// B/E/BE/U and their complements. Only the ST(i) selector is varied below.
const FORMS: [[u8; 2]; 8] = [
    [0xda, 0xc1],
    [0xda, 0xc9],
    [0xda, 0xd1],
    [0xda, 0xd9],
    [0xdb, 0xc1],
    [0xdb, 0xc9],
    [0xdb, 0xd1],
    [0xdb, 0xd9],
];
#[derive(Clone, Copy)]
struct Flags {
    carry: bool,
    zero: bool,
    parity: bool,
    sign: bool,
    overflow: bool,
    rax: u64,
}
const SET: Flags = Flags {
    carry: true,
    zero: true,
    parity: true,
    sign: false,
    overflow: false,
    rax: CANARY,
};
const CLEAR: Flags = Flags {
    carry: false,
    zero: false,
    parity: false,
    sign: false,
    overflow: false,
    rax: 2,
};
const OVERFLOW: Flags = Flags {
    carry: false,
    zero: false,
    parity: true,
    sign: true,
    overflow: true,
    rax: 0x8000_0000_0000_0000,
};
fn taken(index: usize, flags: Flags) -> bool {
    match index {
        0 => flags.carry,
        1 => flags.zero,
        2 => flags.carry || flags.zero,
        3 => flags.parity,
        4 => !flags.carry,
        5 => !flags.zero,
        6 => !flags.carry && !flags.zero,
        _ => !flags.parity,
    }
}
impl Code {
    fn push80(&mut self, value: (u64, u16)) {
        self.word(16, value.0);
        self.word(24, u64::from(value.1));
        self.bytes.extend([0xdb, 0x6c, 0x24, 16]);
    }
    fn control(&mut self, value: u16) {
        self.word(96, u64::from(value));
        self.bytes.extend([0xd9, 0x6c, 0x24, 96]);
    }
    fn seed(&mut self, flags: Flags) {
        self.reg(
            0,
            if flags.overflow {
                0x7fff_ffff_ffff_ffff
            } else {
                flags.rax
            },
        );
        self.reg(10, gpr_pattern(10));
        if flags.overflow {
            self.bytes.extend([0x48, 0x83, 0xc0, 1]);
        } else if flags.zero {
            self.bytes.extend([0x48, 0x39, 0xc0, 0xf9]);
        } else {
            self.bytes.extend([0x48, 0x83, 0xf8, 0]);
        } // CMP RAX,0: 2 has odd parity.
    }
    fn preserved_flags(&mut self, flags: Flags) {
        self.stack(0x89, 0, 176);
        self.stack(0x89, 10, 184);
        for (set, when_set, when_clear) in [
            (flags.carry, 0x83, 0x82),
            (flags.zero, 0x85, 0x84),
            (flags.parity, 0x8b, 0x8a),
            (flags.sign, 0x89, 0x88),
            (flags.overflow, 0x81, 0x80),
        ] {
            self.bad_if(if set { when_set } else { when_clear });
        }
        self.check(176, flags.rax);
        self.check(184, gpr_pattern(10));
        // AF and all non-branch-readable bits have full-state platform consumers.
    }
    fn status(&mut self, expected: u16) {
        self.word(64, CANARY);
        self.bytes.extend([0xdd, 0x7c, 0x24, 64]);
        self.check(64, (CANARY & !0xffff) | u64::from(expected));
        self.word(72, CANARY);
        self.bytes.extend([0xd9, 0x7c, 0x24, 72]);
        self.check(72, (CANARY & !0xffff) | u64::from(CONTROL));
    }
    fn pair(&mut self, index: usize, source: (u64, u16), destination: (u64, u16), flags: Flags) {
        self.begin(ZERO);
        self.push80(source);
        self.push80(destination);
        self.seed(flags);
        self.bytes.extend(FORMS[index]);
        self.preserved_flags(flags);
        self.status(0x3000); // No push/pop or numerical exception, even for true80 denormal.
        self.flags();
        self.pop80(
            if taken(index, flags) {
                source
            } else {
                destination
            },
            0x3800,
        );
        self.flags();
        self.pop80(source, 0); // The physical source remains present and unchanged.
    }
}
fn first_fcmovne() -> Input {
    let mut p = Code::new(ENTRY);
    p.pair(5, (0x8000_0000_0000_0003, 0x3fff), ONE, CLEAR);
    p.pair(5, (0x8000_0000_0000_0003, 0x3fff), ONE, SET);
    image(&p.finish(42), None)
}
fn register_matrix(start: usize) -> Input {
    let pairs = [
        ((0x8000_0000_0000_0001, 0x3fff), ONE),
        (ZERO, (0, 0x8000)),
        (INF, ONE),
        (QUIET, ONE),
        ((1, 0), ONE),
        ((INF.0, 0xffff), INF),
        (
            (0xc000_0000_0000_0000, 0xc000),
            (0xc000_0000_0000_0000, 0x4000),
        ),
        ((0, 0x8000), (0x8000_0000_0000_0003, 0x3fff)),
    ];
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for (index, (source, destination)) in pairs.into_iter().enumerate().skip(start).take(4) {
        for flags in [SET, CLEAR] {
            p.pair(index, source, destination, flags);
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn alias_and_deep_sources() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for (index, form) in FORMS.into_iter().enumerate() {
        p.begin(ZERO);
        p.push80((0x8000_0000_0000_0005, 0x3fff));
        let flags = if index < 4 { SET } else { CLEAR };
        p.seed(flags);
        p.bytes.extend([form[0], form[1] - 1]); // ST0 self-alias.
        p.preserved_flags(flags);
        p.status(0x3800);
        p.flags();
        p.pop80((0x8000_0000_0000_0005, 0x3fff), 0);
    }
    for flags in [SET, CLEAR] {
        p.begin(ZERO);
        let source = (0x8000_0000_0000_0007, 0x3fff);
        p.push80(source);
        for _ in 0..6 {
            p.push80(ONE);
        }
        p.push80(ZERO);
        p.seed(flags);
        p.bytes.extend([0xdb, 0xcf]); // FCMOVNE ST0,ST7.
        p.preserved_flags(flags);
        p.status(0);
        p.flags();
        p.pop80(if taken(5, flags) { source } else { ZERO }, 0x0800);
        for top in 2..8 {
            p.flags();
            p.pop80(ONE, top << 11);
        }
        p.flags();
        p.pop80(source, 0);
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn masked_empty_operands() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for flags in [SET, CLEAR] {
        for missing in 0..3 {
            p.begin(ZERO);
            if missing == 1 {
                p.push80(ONE);
            } // ST0 valid, ST1 empty.
            if missing == 2 {
                p.push80(ONE);
                p.push80(ZERO);
                p.bytes.extend([0xdd, 0xc0]);
            } // FFREE ST0.
            p.seed(flags);
            p.bytes.extend(FORMS[5]);
            p.preserved_flags(flags);
            let top = if missing == 0 {
                0
            } else if missing == 1 {
                0x3800
            } else {
                0x3000
            };
            p.status(top | 0x41);
            p.flags();
            p.pop80(INDEFINITE, ((top + 0x800) & 0x3800) | 0x41);
            if missing == 2 {
                p.flags();
                p.pop80(ONE, 0x41);
            }
            if missing == 1 {
                p.flags();
                p.pop80(INDEFINITE, 0x0841);
            } // Source stayed empty, not repaired.
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn c1_and_overflow_flags() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for flags in [SET, CLEAR, OVERFLOW] {
        p.begin(ZERO);
        for _ in 0..9 {
            p.bytes.extend([0xd9, 0xe8]);
        }
        p.status(0x3a41);
        p.seed(flags);
        p.bytes.extend(FORMS[5]);
        p.preserved_flags(flags);
        p.status(0x3a41); // Normal move retains the genuinely seeded C1=1 and sticky SF/IE.
        p.flags();
        p.pop80(if taken(5, flags) { ONE } else { INDEFINITE }, 0x41);
        p.flags();
        p.pop80(ONE, 0x0841);
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn float_store_gateway() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    let value = (0xc000_0000_0000_0000, 0x3fff); // Exactly 1.5, independent of RC and PC.
    for (wide, popping) in [(false, false), (false, true), (true, false), (true, true)] {
        p.begin(ZERO);
        p.push80(value);
        p.word(48, CANARY);
        p.word(56, CANARY);
        p.flags();
        p.bytes.extend([
            if wide { 0xdd } else { 0xd9 },
            if popping { 0x5c } else { 0x54 },
            0x24,
            49,
        ]);
        p.preserved_flags(SET);
        let expected = if wide {
            0x3ff8_0000_0000_0000_u64
        } else {
            0x3fc0_0000
        };
        let raw = expected.to_le_bytes();
        let mut destination = [0xa5; 16];
        let width = if wide { 8 } else { 4 };
        destination[1..1 + width].copy_from_slice(&raw[..width]);
        p.check(48, u64::from_le_bytes(destination[..8].try_into().unwrap()));
        p.check(56, u64::from_le_bytes(destination[8..].try_into().unwrap()));
        p.status(if popping { 0 } else { 0x3800 });
        p.flags();
        p.pop80(
            if popping { INDEFINITE } else { value },
            if popping { 0x0841 } else { 0 },
        );
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn threaded_moves() -> Input {
    let held = (0x8000_0000_0000_0003, 0x3fff);
    let mut main = Code::new(ENTRY);
    main.begin(ZERO);
    main.push80(held);
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
    main.pop80(held, 0);
    let mut child = Code::new(CHILD);
    child.seed_context();
    child.begin(ZERO);
    child.push80(held);
    child.push80(ONE);
    child.seed(CLEAR);
    child.bytes.extend(FORMS[5]);
    child.preserved_flags(CLEAR);
    child.status(0x3000);
    child.call(2);
    child.expect(0, 1);
    // Real SYSCALL completion owns RCX/R11; reseed only those volatile baselines.
    child.reg(1, gpr_pattern(1));
    child.reg(11, gpr_pattern(11));
    child.seed(SET);
    child.bytes.extend(FORMS[7]);
    child.preserved_flags(SET);
    child.status(0x3000);
    child.flags();
    child.pop80(held, 0x3800);
    child.flags();
    child.pop80(held, 0);
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
fn native_x87_conditional_moves_all_conditions_exact_sources_flags_and_thread_context_have_integer_consumers()
 {
    let programs = [
        first_fcmovne(),
        register_matrix(0),
        register_matrix(4),
        alias_and_deep_sources(),
        masked_empty_operands(),
        c1_and_overflow_flags(),
        float_store_gateway(),
        threaded_moves(),
    ];
    let mut completed = 0;
    for (index, source) in programs.iter().enumerate() {
        let (status, fields, text) = run(source);
        assert_eq!(status, 0, "program={index}: {text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["code"], "outer-return");
        assert_eq!(
            fields["fp_status"],
            if index == 0 || index == 7 { "0" } else { "4" }
        );
        assert_eq!(fields["service_stops"], if index == 7 { "3" } else { "0" });
        if index == 7 {
            assert_eq!(fields["scheduling_threads"], "2");
            assert_eq!(fields["scheduling_returned"], "1");
        }
        completed += 1;
    }
    assert_eq!(completed, 8);
    println!(
        "x87_conditional_move_native_available=true x87_conditional_move_native_programs={completed} x87_conditional_move_native_returns={completed}"
    );
}
#[test]
fn native_x87_conditional_move_false_empty_and_pending_unmasked_exceptions_refuse_without_return() {
    let mut completed = 0;
    for kind in 0..5 {
        let mut p = Code::new(ENTRY);
        p.begin(ZERO);
        match kind {
            0 => {
                p.control(0x037e);
                p.seed(SET);
                p.bytes.extend(FORMS[5]);
            } // False, both empty.
            1 => {
                p.push80(ONE);
                p.push80(ZERO);
                p.bytes.extend([0xdd, 0xc0]);
                p.control(0x037e);
                p.seed(SET);
                p.bytes.extend(FORMS[5]);
            }
            2 => {
                p.push80(ONE);
                p.control(0x037e);
                p.seed(SET);
                p.bytes.extend(FORMS[5]);
            }
            3 => {
                p.seed(SET);
                p.bytes.extend(FORMS[5]);
                p.push80(ONE);
                p.control(0x037e);
                p.seed(SET);
                p.bytes.extend(FORMS[5]);
            }
            _ => {
                p.control(0x037e);
                p.seed(CLEAR);
                p.bytes.extend(FORMS[5]);
            }
        }
        let source = image(&p.finish(42), None);
        let (status, fields, text) = run(&source);
        refused(status, &fields, &text);
        completed += 1;
    }
    assert_eq!(completed, 5);
    println!(
        "x87_conditional_move_refusal_native_available=true x87_conditional_move_refusal_native_programs={completed} x87_conditional_move_refusal_native_returns=0"
    );
}
