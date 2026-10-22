// SPDX-License-Identifier: GPL-2.0-only
//! Original checked x87 arithmetic consumers. Integer guest branches consume
//! independently specified 80-bit literals; no host arithmetic oracle is used.
//! Guest context continuity is distinct from same-process host restoration,
//! which the platform boundary fixtures verify independently.
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

// This independent test-only gate queries capability, never host arithmetic.
// Every optional leaf is guarded by its reported maximum. Unknown vendors
// require an actual typed refusal; availability is never invented for them.
fn arithmetic_host() -> bool {
    use std::arch::x86_64::__cpuid;
    // CPUID is safe on x86-64. Every optional leaf follows its reported maximum.
    let root = __cpuid(0);
    if root.eax < 1 {
        return false;
    }
    let features = __cpuid(1);
    if features.edx & 0x0100_0001 != 0x0100_0001 {
        return false;
    }
    let mut vendor = [0u8; 12];
    vendor[..4].copy_from_slice(&root.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&root.edx.to_le_bytes());
    vendor[8..].copy_from_slice(&root.ecx.to_le_bytes());
    vendor == *b"GenuineIntel"
        || (vendor == *b"AuthenticAMD"
            && __cpuid(0x8000_0000).eax >= 0x8000_0008
            && __cpuid(0x8000_0008).ebx & 4 != 0)
}

const TWELVE: (u64, u16) = (0xc000_0000_0000_0000, 0x4002);
const THREE: (u64, u16) = (0xc000_0000_0000_0000, 0x4000);
const ZERO: (u64, u16) = (0, 0);
const INF: (u64, u16) = (0x8000_0000_0000_0000, 0x7fff);
#[derive(Clone, Copy)]
struct Family {
    st0: u8,
    sti: u8,
    group: u8,
    result_st0: (u64, u16),
    result_sti: (u64, u16),
}
// FSUBRP is deliberately first. ST0=12 and ST1=3 make its noncommutative
// result 9 distinguishable from FSUBP's -9 and from a missing pop.
const FAMILIES: [Family; 6] = [
    Family {
        st0: 0xe8,
        sti: 0xe0,
        group: 5,
        result_st0: (0x9000_0000_0000_0000, 0xc002),
        result_sti: (0x9000_0000_0000_0000, 0x4002),
    },
    Family {
        st0: 0xc0,
        sti: 0xc0,
        group: 0,
        result_st0: (0xf000_0000_0000_0000, 0x4002),
        result_sti: (0xf000_0000_0000_0000, 0x4002),
    },
    Family {
        st0: 0xc8,
        sti: 0xc8,
        group: 1,
        result_st0: (0x9000_0000_0000_0000, 0x4004),
        result_sti: (0x9000_0000_0000_0000, 0x4004),
    },
    Family {
        st0: 0xe0,
        sti: 0xe8,
        group: 4,
        result_st0: (0x9000_0000_0000_0000, 0x4002),
        result_sti: (0x9000_0000_0000_0000, 0xc002),
    },
    Family {
        st0: 0xf0,
        sti: 0xf8,
        group: 6,
        result_st0: (0x8000_0000_0000_0000, 0x4001),
        result_sti: (0x8000_0000_0000_0000, 0x3ffd),
    },
    Family {
        st0: 0xf8,
        sti: 0xf0,
        group: 7,
        result_st0: (0x8000_0000_0000_0000, 0x3ffd),
        result_sti: (0x8000_0000_0000_0000, 0x4001),
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
    fn arithmetic_status(&mut self, status: u16, control: u16) {
        self.stack(0x89, 0, 176);
        self.stack(0x89, 10, 184);
        self.check_flags();
        self.check(176, CANARY);
        self.check(184, gpr_pattern(10));
        self.word(64, CANARY);
        self.bytes.extend([0xdd, 0x7c, 0x24, 64]);
        self.check(64, (CANARY & !0xffff) | u64::from(status));
        self.word(72, CANARY);
        self.bytes.extend([0xd9, 0x7c, 0x24, 72]);
        self.check(72, (CANARY & !0xffff) | u64::from(control));
    }
}
fn probe_program() -> Input {
    let mut p = Code::new(ENTRY);
    p.begin(ZERO);
    p.push80(THREE);
    p.push80(TWELVE);
    p.flags();
    p.bytes.extend([0xde, 0xe1]); // FSUBRP ST1,ST0:12-3, then pop.
    p.arithmetic_status(0x3800, CONTROL);
    p.flags();
    p.pop80((0x9000_0000_0000_0000, 0x4002), 0);
    image(&p.finish(42), None)
}
fn register_matrix(mode: u8) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for family in FAMILIES {
        p.begin(ZERO);
        p.push80(THREE);
        p.push80(TWELVE);
        p.flags();
        let (opcode, operand) = match mode {
            0 => (0xd8, family.st0 | 1),
            1 => (0xdc, family.sti | 1),
            2 => (0xde, family.sti | 1),
            _ => unreachable!(),
        };
        p.bytes.extend([opcode, operand]);
        p.arithmetic_status(if mode == 2 { 0x3800 } else { 0x3000 }, CONTROL);
        p.flags();
        match mode {
            0 => {
                p.pop80(family.result_st0, 0x3800);
                p.flags();
                p.pop80(THREE, 0);
            }
            1 => {
                p.pop80(TWELVE, 0x3800);
                p.flags();
                p.pop80(family.result_sti, 0);
            }
            _ => p.pop80(family.result_sti, 0),
        }
    }
    if mode == 0 {
        for (opcode, result) in [
            (0xc0, (0xc000_0000_0000_0000, 0x4003)), // 12+12=24.
            (0xc8, (0x9000_0000_0000_0000, 0x4006)), // 12*12=144.
            (0xe8, ZERO),                            // 12-12=+0 under nearest rounding.
        ] {
            p.begin(ZERO);
            p.push80(TWELVE);
            p.flags();
            p.bytes.extend([0xd8, opcode]);
            p.arithmetic_status(0x3800, CONTROL);
            p.flags();
            p.pop80(result, 0);
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn memory_matrix(wide: bool) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    for family in FAMILIES {
        p.begin(ZERO);
        p.push80(TWELVE);
        p.word(
            16,
            if wide {
                0x4008_0000_0000_0000
            } else {
                0xa5a5_a5a5_4040_0000
            },
        );
        p.flags();
        p.bytes.extend([
            if wide { 0xdc } else { 0xd8 },
            0x44 | (family.group << 3),
            0x24,
            16,
        ]);
        p.arithmetic_status(0x3800, CONTROL);
        p.flags();
        p.pop80(family.result_st0, 0);
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn precision_matrix(negative: bool) -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    // 1+2^-p is exactly halfway between adjacent values at precision p.
    // These explicit significand steps are independent integer ISA literals.
    for (pc, epsilon_exp, step) in [
        (0_u16, 0x3fe7, 1_u64 << 40),
        (2, 0x3fca, 1 << 11),
        (3, 0x3fbf, 1),
    ] {
        for rc in 0..4_u16 {
            let sign = if negative { 0x8000 } else { 0 };
            let control = 0x007f | (pc << 8) | (rc << 10);
            p.begin(ZERO);
            p.push80((ONE.0, ONE.1 | sign));
            p.push80((ONE.0, epsilon_exp | sign));
            p.control(control);
            p.flags();
            p.bytes.extend([0xde, 0xc1]); // FADDP ST1,ST0.
            let increment = (!negative && rc == 2) || (negative && rc == 1);
            p.arithmetic_status(0x3820 | if increment { 0x0200 } else { 0 }, control);
            // The arithmetic result is already rounded; m80 transfer itself
            // has no narrowing. Check actual control before restoring the seed.
            p.control(CONTROL);
            p.flags();
            p.pop80(
                (ONE.0 + if increment { step } else { 0 }, ONE.1 | sign),
                0x20,
            );
        }
    }
    p.check_context();
    image(&p.finish(42), None)
}
fn masked_arithmetic() -> Input {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.begin(ZERO);
    p.push80(ONE);
    p.push80(ZERO);
    p.flags();
    p.bytes.extend([0xde, 0xf9]); // 1/0=+Inf, maskedZE.
    p.arithmetic_status(0x3804, CONTROL);
    p.flags();
    p.pop80(INF, 4);
    p.begin(ZERO);
    p.push80(ZERO);
    p.push80(ZERO);
    p.flags();
    p.bytes.extend([0xde, 0xf9]); // 0/0=negative indefinite, maskedIE.
    p.arithmetic_status(0x3801, CONTROL);
    p.flags();
    p.pop80(INDEFINITE, 1);
    p.begin(ZERO);
    p.push80(ONE);
    p.word(16, 0xa5a5_a5a5_7f80_0001);
    p.flags();
    p.bytes.extend([0xd8, 0x44, 0x24, 16]); // A lone memory SNaN quiets and raises IE.
    p.arithmetic_status(0x3801, CONTROL);
    p.flags();
    p.pop80((0xc000_0100_0000_0000, 0x7fff), 1);
    p.check_context();
    image(&p.finish(42), None)
}
fn threaded_arithmetic() -> Input {
    let mut main = Code::new(ENTRY);
    main.begin(ZERO);
    main.push80((0x8000_0000_0000_0001, 0x3fff));
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
    main.pop80((0x8000_0000_0000_0001, 0x3fff), 0);
    let mut child = Code::new(CHILD);
    child.seed_context();
    child.begin(ZERO);
    child.push80(THREE);
    child.push80(TWELVE);
    child.flags();
    child.bytes.extend([0xde, 0xe1]);
    child.arithmetic_status(0x3800, CONTROL);
    child.call(2);
    child.expect(0, 1);
    child.reg(1, gpr_pattern(1));
    child.reg(11, gpr_pattern(11));
    child.flags();
    child.pop80((0x9000_0000_0000_0000, 0x4002), 0);
    child.check_context();
    image(&main.finish(42), Some(&child.finish(77)))
}
fn refused(status: i32, fields: &HashMap<String, String>, text: &str, category: &str) {
    assert_ne!(status, 0, "{text}");
    assert_eq!(fields["code"], "native-dispatch");
    assert_eq!(fields["runtime_failure"], category);
    assert!(!fields.contains_key("value"));
    assert_eq!(fields["service_stops"], "0");
}

#[test]
fn native_x87_arithmetic_reverse_pop_all_forms_precision_rounding_and_thread_context_have_integer_consumers()
 {
    let supported = arithmetic_host();
    let first = probe_program();
    let (status, fields, text) = run(&first);
    if !supported {
        refused(status, &fields, &text, "vector-form");
        println!(
            "x87_arithmetic_native_available=false x87_arithmetic_native_programs=1 x87_arithmetic_native_returns=0"
        );
        return;
    }
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
    let programs = [
        register_matrix(2),
        register_matrix(0),
        register_matrix(1),
        memory_matrix(false),
        memory_matrix(true),
        precision_matrix(false),
        precision_matrix(true),
        masked_arithmetic(),
        threaded_arithmetic(),
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
        "x87_arithmetic_native_available=true x87_arithmetic_native_programs={completed} x87_arithmetic_native_returns={completed}"
    );
}

#[test]
fn native_x87_arithmetic_unmasked_pending_and_reserved_precision_never_publish_a_return() {
    if !arithmetic_host() {
        let source = probe_program();
        let (status, fields, text) = run(&source);
        refused(status, &fields, &text, "vector-form");
        println!(
            "x87_arithmetic_refusal_native_available=false x87_arithmetic_refusal_native_programs=1 x87_arithmetic_refusal_native_returns=0"
        );
        return;
    }
    let mut completed = 0;
    for kind in 0..4 {
        let mut p = Code::new(ENTRY);
        p.begin(ZERO);
        if kind == 2 {
            p.push80(THREE);
            p.push80(TWELVE);
            p.control(0x017f);
            p.flags();
            p.bytes.extend([0xde, 0xe1]);
        } else {
            p.push80(if kind == 3 { ZERO } else { ONE });
            p.push80(ZERO);
            if kind == 1 {
                p.flags();
                p.bytes.extend([0xde, 0xf9]); // Masked 1/0 leaves a pending ZE.
                p.control(0x037b);
                p.flags();
                p.bytes.extend([0xd8, 0x44, 0x24, 16]);
            } else {
                p.control(if kind == 3 { 0x037e } else { 0x037b });
                p.flags();
                p.bytes.extend([0xde, 0xf9]);
            }
        }
        let source = image(&p.finish(42), None);
        let (status, fields, text) = run(&source);
        refused(
            status,
            &fields,
            &text,
            if kind == 2 {
                "vector-form"
            } else {
                "x87-exception"
            },
        );
        completed += 1;
    }
    assert_eq!(completed, 4);
    println!(
        "x87_arithmetic_refusal_native_available=true x87_arithmetic_refusal_native_programs={completed} x87_arithmetic_refusal_native_returns=0"
    );
}
