// SPDX-License-Identifier: GPL-2.0-only
//! Original integer consumers for checked x87 comparisons. Every guest outcome
//! is consumed before return; no host floating-point comparison oracle is used.
//! Guest flag results are checked independently of the host instruction set.
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
const THREE: (u64, u16) = (0xc000_0000_0000_0000, 0x4000);
const TWELVE: (u64, u16) = (0xc000_0000_0000_0000, 0x4002);
const INF: (u64, u16) = (0x8000_0000_0000_0000, 0x7fff);
const QUIET: (u64, u16) = (0xc000_0000_0000_0055, 0x7fff);
#[derive(Clone, Copy)]
enum Relation {
    Greater,
    Less,
    Equal,
    Unordered,
}
impl Relation {
    fn bits(self) -> u16 {
        match self {
            Self::Greater => 0,
            Self::Less => 0x0100,
            Self::Equal => 0x4000,
            Self::Unordered => 0x4500,
        }
    }
    fn flags(self) -> (bool, bool, bool) {
        match self {
            Self::Greater => (false, false, false),
            Self::Less => (false, false, true),
            Self::Equal => (true, false, false),
            Self::Unordered => (true, true, true),
        }
    }
}
#[derive(Clone, Copy)]
struct Case {
    left: (u64, u16),
    right: (u64, u16),
    relation: Relation,
}
const CASES: [Case; 4] = [
    Case {
        left: (0x8000_0000_0000_0001, 0x3fff),
        right: ONE,
        relation: Relation::Greater,
    },
    Case {
        left: (0, 0x8000),
        right: ZERO,
        relation: Relation::Equal,
    },
    Case {
        left: (INF.0, 0xffff),
        right: INF,
        relation: Relation::Less,
    },
    Case {
        left: QUIET,
        right: ONE,
        relation: Relation::Unordered,
    },
];
#[derive(Clone, Copy)]
struct Form {
    bytes: [u8; 2],
    pops: u16,
    unordered: bool,
    integer: bool,
}
const INTEGER: [Form; 4] = [
    Form {
        bytes: [0xdb, 0xe9],
        pops: 0,
        unordered: true,
        integer: true,
    }, // FUCOMI ST0,ST1.
    Form {
        bytes: [0xdb, 0xf1],
        pops: 0,
        unordered: false,
        integer: true,
    },
    Form {
        bytes: [0xdf, 0xe9],
        pops: 1,
        unordered: true,
        integer: true,
    },
    Form {
        bytes: [0xdf, 0xf1],
        pops: 1,
        unordered: false,
        integer: true,
    },
];
const STATUS: [Form; 6] = [
    Form {
        bytes: [0xd8, 0xd1],
        pops: 0,
        unordered: false,
        integer: false,
    },
    Form {
        bytes: [0xd8, 0xd9],
        pops: 1,
        unordered: false,
        integer: false,
    },
    Form {
        bytes: [0xde, 0xd9],
        pops: 2,
        unordered: false,
        integer: false,
    },
    Form {
        bytes: [0xdd, 0xe1],
        pops: 0,
        unordered: true,
        integer: false,
    },
    Form {
        bytes: [0xdd, 0xe9],
        pops: 1,
        unordered: true,
        integer: false,
    },
    Form {
        bytes: [0xda, 0xe9],
        pops: 2,
        unordered: true,
        integer: false,
    },
];
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
    fn seed_integer_flags(&mut self) {
        self.reg(0, 0x7fff_ffff_ffff_ffff);
        self.reg(10, gpr_pattern(10));
        self.bytes.extend([0x48, 0x83, 0xc0, 1]); // ADD: OF=SF=AF=1 before COMI.
    }
    fn integer_flags(&mut self, relation: Relation) {
        self.stack(0x89, 0, 176);
        self.stack(0x89, 10, 184);
        let (zero, parity, carry) = relation.flags();
        self.bad_if(if zero { 0x85 } else { 0x84 });
        self.bad_if(if parity { 0x8b } else { 0x8a });
        self.bad_if(if carry { 0x83 } else { 0x82 });
        self.bad_if(0x80);
        self.bad_if(0x88); // JO/JS fail: COMI clears seeded OF/SF.
        self.check(176, 0x8000_0000_0000_0000);
        self.check(184, gpr_pattern(10));
        // Exact AF clearing and every unrelated RFLAGS bit are direct platform-state
        // fixtures; this guest profile has no admitted LAHF/PUSHFQ observer.
    }
    fn data_flags(&mut self) {
        self.stack(0x89, 0, 176);
        self.stack(0x89, 10, 184);
        self.check_flags();
        self.check(176, CANARY);
        self.check(184, gpr_pattern(10));
    }
    fn status(&mut self, expected: u16) {
        self.word(64, CANARY);
        self.bytes.extend([0xdd, 0x7c, 0x24, 64]);
        self.check(64, (CANARY & !0xffff) | u64::from(expected));
        self.word(72, CANARY);
        self.bytes.extend([0xd9, 0x7c, 0x24, 72]);
        self.check(72, (CANARY & !0xffff) | u64::from(CONTROL));
    }
    fn pair(&mut self, form: Form, case: Case) {
        self.begin(ZERO);
        self.push80(case.right);
        self.push80(case.left);
        if form.integer {
            self.seed_integer_flags();
        } else {
            self.flags();
        }
        self.bytes.extend(form.bytes);
        let invalid = u16::from(!form.unordered && matches!(case.relation, Relation::Unordered));
        let conditions = if form.integer {
            0
        } else {
            case.relation.bits()
        };
        if form.integer {
            self.integer_flags(case.relation);
        } else {
            self.data_flags();
        }
        let top = match form.pops {
            0 => 0x3000,
            1 => 0x3800,
            _ => 0,
        };
        self.status(top | conditions | invalid);
        self.flags();
        match form.pops {
            0 => {
                self.pop80(case.left, 0x3800 | conditions | invalid);
                self.flags();
                self.pop80(case.right, conditions | invalid);
            }
            1 => self.pop80(case.right, conditions | invalid),
            _ => self.pop80(INDEFINITE, 0x0841 | conditions | invalid), // Real empty tag after two pops.
        }
    }
}
fn first_fucomi() -> Input {
    let mut p = Code::new(ENTRY);
    p.pair(
        INTEGER[0],
        Case {
            left: TWELVE,
            right: THREE,
            relation: Relation::Greater,
        },
    );
    image(&p.finish(42), None)
}
fn register_matrix(forms: &[Form]) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for form in forms {
        for case in CASES {
            p.pair(*form, case);
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn memory_matrix(wide: bool) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    let cases = [
        (
            TWELVE,
            0x4040_0000_u64,
            0x4008_0000_0000_0000_u64,
            Relation::Greater,
            0_u16,
        ),
        ((0, 0x8000), 0, 0, Relation::Equal, 0),
        (
            (INF.0, 0xffff),
            0x7f80_0000,
            0x7ff0_0000_0000_0000,
            Relation::Less,
            0,
        ),
        (
            ONE,
            0x7fc0_0055,
            0x7ff8_0000_0000_0055,
            Relation::Unordered,
            1,
        ),
        (
            ONE,
            0x7f80_0001,
            0x7ff0_0000_0000_0001,
            Relation::Unordered,
            1,
        ),
        (ONE, 1, 1, Relation::Greater, 2),
        (QUIET, 1, 1, Relation::Unordered, 1), // Invalid NaN suppresses operand DE.
    ];
    for popping in [false, true] {
        for (left, word32, word64, relation, exception) in cases {
            p.begin(ZERO);
            p.push80(left);
            let word = if wide {
                word64
            } else {
                word32 | 0xa5a5_a5a5_0000_0000
            };
            p.word(16, word);
            p.flags();
            p.bytes.extend([
                if wide { 0xdc } else { 0xd8 },
                if popping { 0x5c } else { 0x54 },
                0x24,
                16,
            ]);
            p.data_flags();
            let conditions = relation.bits();
            p.status((if popping { 0 } else { 0x3800 }) | conditions | exception);
            p.check(16, word);
            p.flags();
            if popping {
                p.pop80(INDEFINITE, 0x0841 | conditions | exception);
            } else {
                p.pop80(left, conditions | exception);
            }
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn test_zero_matrix() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for (value, relation, exception) in [
        (THREE, Relation::Greater, 0_u16),
        ((THREE.0, 0xc000), Relation::Less, 0),
        (ZERO, Relation::Equal, 0),
        ((0, 0x8000), Relation::Equal, 0),
        (INF, Relation::Greater, 0),
        ((INF.0, 0xffff), Relation::Less, 0),
        (QUIET, Relation::Unordered, 1),
    ] {
        p.begin(ZERO);
        p.push80(value);
        p.flags();
        p.bytes.extend([0xd9, 0xe4]);
        p.data_flags();
        p.status(0x3800 | relation.bits() | exception);
        p.flags();
        p.pop80(value, relation.bits() | exception);
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn masked_stack_matrix() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for form in [INTEGER[0], INTEGER[2], STATUS[0], STATUS[2]] {
        p.begin(ZERO);
        if form.integer {
            p.seed_integer_flags();
        } else {
            p.flags();
        }
        p.bytes.extend(form.bytes);
        if form.integer {
            p.integer_flags(Relation::Unordered);
        } else {
            p.data_flags();
        }
        let conditions = if form.integer { 0 } else { 0x4500 };
        p.status((form.pops << 11) | conditions | 0x41);
        p.flags();
        p.pop80(INDEFINITE, ((form.pops + 1) << 11) | conditions | 0x41);
    }
    // Existing masked FLD1 overflow supplies a genuine set C1 without a
    // new state-import instruction. COMI preserves it; ordinary data stores
    // below clear it while consuming the real destination and exact pop.
    for form in [INTEGER[0], INTEGER[2]] {
        p.begin(ZERO);
        for _ in 0..9 {
            p.bytes.extend([0xd9, 0xe8]);
        }
        p.status(0x3a41);
        p.seed_integer_flags();
        p.bytes.extend(form.bytes);
        p.integer_flags(Relation::Unordered);
        p.status(if form.pops == 0 { 0x3a41 } else { 0x0241 });
        p.flags();
        if form.pops == 0 {
            p.pop80(INDEFINITE, 0x0041);
        } else {
            p.pop80(ONE, 0x0841);
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn threaded_comparison() -> Input {
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
    child.push80(ONE);
    child.push80((0x8000_0000_0000_0001, 0x3fff));
    child.seed_integer_flags();
    child.bytes.extend(INTEGER[0].bytes);
    child.integer_flags(Relation::Greater);
    child.status(0x3000);
    child.call(2);
    child.expect(0, 1);
    child.reg(1, gpr_pattern(1));
    child.reg(11, gpr_pattern(11));
    child.flags();
    child.pop80((0x8000_0000_0000_0001, 0x3fff), 0x3800);
    child.flags();
    child.pop80(ONE, 0);
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
fn native_x87_comparison_flags_status_pops_memory_and_current_thread_context_have_integer_consumers()
 {
    let first = first_fucomi();
    let (status, fields, text) = run(&first);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
    let programs = [
        register_matrix(&INTEGER[..2]),
        register_matrix(&INTEGER[2..]),
        register_matrix(&STATUS[..3]),
        register_matrix(&STATUS[3..]),
        memory_matrix(false),
        memory_matrix(true),
        test_zero_matrix(),
        masked_stack_matrix(),
        threaded_comparison(),
    ];
    let mut completed = 1;
    for (index, source) in programs.iter().enumerate() {
        let (status, fields, text) = run(source);
        assert_eq!(status, 0, "program={index}: {text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["code"], "outer-return");
        assert_eq!(fields["fp_status"], if index == 8 { "0" } else { "4" });
        assert_eq!(fields["service_stops"], if index == 8 { "3" } else { "0" });
        if index == 8 {
            assert_eq!(fields["scheduling_threads"], "2");
            assert_eq!(fields["scheduling_returned"], "1");
        }
        completed += 1;
    }
    assert_eq!(completed, 10);
    println!(
        "x87_comparison_native_available=true x87_comparison_native_programs={completed} x87_comparison_native_returns={completed}"
    );
}
#[test]
fn native_x87_comparison_pending_and_unmasked_invalid_denormal_and_empty_stack_refuse_without_return()
 {
    let mut completed = 0;
    for kind in 0..5 {
        let mut p = Code::new(ENTRY);
        p.begin(ZERO);
        match kind {
            0 => {
                p.push80(ONE);
                p.push80(QUIET);
                p.control(0x037e);
                p.seed_integer_flags();
                p.bytes.extend(INTEGER[1].bytes);
            }
            1 => {
                p.push80(ONE);
                p.word(16, 0xa5a5_a5a5_7f80_0001);
                p.control(0x037e);
                p.flags();
                p.bytes.extend([0xd8, 0x54, 0x24, 16]);
            }
            2 => {
                p.push80(ONE);
                p.word(16, 0xa5a5_a5a5_0000_0001);
                p.control(0x037d);
                p.flags();
                p.bytes.extend([0xd8, 0x54, 0x24, 16]);
            }
            3 => {
                p.push80(ONE);
                p.word(16, 0xa5a5_a5a5_7f80_0001);
                p.flags();
                p.bytes.extend([0xd8, 0x54, 0x24, 16]);
                p.control(0x037e);
                p.seed_integer_flags();
                p.bytes.extend([0xdb, 0xe8]);
            }
            _ => {
                p.control(0x037e);
                p.seed_integer_flags();
                p.bytes.extend(INTEGER[2].bytes);
            }
        }
        let source = image(&p.finish(42), None);
        let (status, fields, text) = run(&source);
        refused(status, &fields, &text);
        completed += 1;
    }
    assert_eq!(completed, 5);
    println!(
        "x87_comparison_refusal_native_available=true x87_comparison_refusal_native_programs={completed} x87_comparison_refusal_native_returns=0"
    );
}
