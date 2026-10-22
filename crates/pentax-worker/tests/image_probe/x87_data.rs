// SPDX-License-Identifier: GPL-2.0-only
//! Original x87 data consumers. Exact 80-bit literals are checked by integer
//! guest loads/branches; no host floating-point oracle or commercial input.
use super::*;

const ENTRY: i32 = 0x1000;
const CHILD: i32 = 0x5000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const ONE: (u64, u16) = (0x8000_0000_0000_0000, 0x3fff);
const INDEFINITE: (u64, u16) = (0xc000_0000_0000_0000, 0xffff);
const CONTROL: u16 = 0x0b7f;
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

fn memory_matrix(width: u8) -> Vec<u8> {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    // Exact literal expected 80-bit encodings, not a production conversion oracle.
    type TransferCase = ((u64, u16), (u64, u16), u16);
    let cases: Vec<TransferCase> = match width {
        4 => vec![
            ((0x3f80_0000, 0), ONE, 0),
            ((0x8000_0000, 0), (0, 0x8000), 0),
            ((1, 0), (0x8000_0000_0000_0000, 0x3f6a), 2),
            ((0xff80_0000, 0), (0x8000_0000_0000_0000, 0xffff), 0),
            ((0x7fc0_0001, 0), (0xc000_0100_0000_0000, 0x7fff), 0),
            ((0x7f80_0001, 0), (0xc000_0100_0000_0000, 0x7fff), 1),
        ],
        8 => vec![
            ((0x3ff0_0000_0000_0000, 0), ONE, 0),
            ((0x8000_0000_0000_0000, 0), (0, 0x8000), 0),
            ((1, 0), (0x8000_0000_0000_0000, 0x3bcd), 2),
            (
                (0xfff0_0000_0000_0000, 0),
                (0x8000_0000_0000_0000, 0xffff),
                0,
            ),
            (
                (0x7ff8_0000_0000_0001, 0),
                (0xc000_0000_0000_0800, 0x7fff),
                0,
            ),
            (
                (0x7ff0_0000_0000_0001, 0),
                (0xc000_0000_0000_0800, 0x7fff),
                1,
            ),
        ],
        10 => vec![
            (
                (0x8000_0000_0000_0001, 0x3fff),
                (0x8000_0000_0000_0001, 0x3fff),
                0,
            ),
            ((0, 0x8000), (0, 0x8000), 0),
            ((1, 0), (1, 0), 0),
            (
                (0x8000_0000_0000_0000, 0xffff),
                (0x8000_0000_0000_0000, 0xffff),
                0,
            ),
            (
                (0xc000_0000_0000_0001, 0x7fff),
                (0xc000_0000_0000_0001, 0x7fff),
                0,
            ),
        ],
        _ => unreachable!(),
    };
    for (input, expected, status) in cases {
        let input = if width == 4 {
            (input.0 | 0xa5a5_a5a5_0000_0000, input.1)
        } else {
            input
        };
        p.begin(input);
        p.bytes.extend(match width {
            4 => [0xd9, 0x44, 0x24, 16],
            8 => [0xdd, 0x44, 0x24, 16],
            _ => [0xdb, 0x6c, 0x24, 16],
        });
        p.pop80(expected, status);
    }
    p.check_context();
    p.finish(42)
}
fn register_moves() -> Vec<u8> {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.begin((0, 0));
    p.bytes
        .extend([0xd9, 0xe8, 0xd9, 0xee, 0xd9, 0xe8, 0xd9, 0xe0]); //1,0,-1.
    p.bytes
        .extend([0xd9, 0xca, 0xd9, 0xc0, 0xdd, 0xd2, 0xdd, 0xd9]); // FXCH2; FLD0; FST2; FSTP1.
    p.bytes.extend([0xd9, 0xca, 0xd9, 0xe1]); // FXCH2; FABS.
    p.pop80(ONE, 0x3000);
    p.flags();
    p.pop80(ONE, 0x3800);
    p.flags();
    p.pop80(ONE, 0);
    p.begin((0, 0));
    p.bytes.extend([0xd9, 0xe8, 0xd9, 0xee, 0xd9, 0xf7]); //1,0; FINCSTP selects retained 1.
    p.pop80(ONE, 0);
    p.bytes.extend([0xd9, 0xf6, 0xd9, 0xf6]); // FDECSTP twice selects retained 0.
    p.flags();
    p.pop80((0, 0), 0x3800);
    p.bytes.extend([0xd9, 0xf7]);
    p.begin((0, 0));
    p.bytes.extend([0xd9, 0xee, 0xd9, 0xe0]);
    p.pop80((0, 0x8000), 0);
    p.begin((0, 0));
    p.bytes.extend([0xd9, 0xee, 0xd9, 0xe0, 0xd9, 0xe1]);
    p.pop80((0, 0), 0);
    p.begin((0, 0));
    p.bytes
        .extend([0xd9, 0xe8, 0xdd, 0xd0, 0xdd, 0xd8, 0xd9, 0xee]);
    p.pop80((0, 0), 0); // FST0 preserves1; FSTP0 pops it before FLDZ.
    p.check_context();
    p.finish(42)
}
fn masked_faults() -> Vec<u8> {
    let mut p = Code::new(ENTRY);
    p.seed_context();
    p.begin((0, 0));
    for _ in 0..9 {
        p.bytes.extend([0xd9, 0xe8]);
    }
    // Read the overflow status before the following store clears C1.
    p.word(64, CANARY);
    p.bytes.extend([0xdd, 0x7c, 0x24, 64]);
    p.check_flags();
    p.check(64, (CANARY & !0xffff) | 0x3a41);
    p.flags();
    p.pop80(INDEFINITE, 0x0041);
    p.begin((0, 0));
    p.pop80(INDEFINITE, 0x0841);
    p.begin((0, 0));
    p.bytes.extend([0xd9, 0xe8, 0xdd, 0xc0]); // FFREE0 removes a real1.
    p.pop80(INDEFINITE, 0x0041);
    p.check_context();
    p.finish(42)
}
fn threaded() -> Input {
    let mut main = Code::new(ENTRY);
    main.begin((ONE.0, ONE.1));
    main.bytes.extend([0xdb, 0x6c, 0x24, 16]);
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
    main.pop80(ONE, 0);
    let mut child = Code::new(CHILD);
    child.seed_context();
    child.begin((0x8000_0000_0000_0001, 0x3fff));
    child.bytes.extend([0xdb, 0x6c, 0x24, 16]);
    child.call(2);
    child.expect(0, 1);
    // SYSCALL deliberately publishes RCX=next_pc and R11=rflags. Establish
    // their post-service baselines before checking the following x87 transfer.
    child.reg(1, gpr_pattern(1));
    child.reg(11, gpr_pattern(11));
    child.flags();
    child.pop80((0x8000_0000_0000_0001, 0x3fff), 0);
    child.check_context();
    image(&main.finish(42), Some(&child.finish(77)))
}

#[test]
fn native_x87_data_literal80_register_masks_and_real_child_context_reach_integer_consumers() {
    let programs = [
        image(&memory_matrix(8), None),
        image(&memory_matrix(4), None),
        image(&memory_matrix(10), None),
        image(&register_moves(), None),
        image(&masked_faults(), None),
        threaded(),
    ];
    let mut completed = 0;
    for (index, source) in programs.iter().enumerate() {
        let (status, fields, text) = run(source);
        assert_eq!(status, 0, "program={index}: {text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["code"], "outer-return");
        assert_eq!(fields["fp_status"], if index == 5 { "0" } else { "4" });
        assert_eq!(fields["service_stops"], if index == 5 { "3" } else { "0" });
        if index == 5 {
            assert_eq!(fields["scheduling_threads"], "2");
            assert_eq!(fields["scheduling_returned"], "1");
        }
        completed += 1;
    }
    assert_eq!(completed, 6);
    println!(
        "x87_data_native_available=true x87_data_native_programs={completed} x87_data_native_returns={completed}"
    );
}

#[test]
fn native_x87_pending_and_new_unmasked_faults_stop_without_publishing_a_return() {
    for pending in [false, true] {
        let mut p = Code::new(ENTRY);
        p.bytes.extend([0xdb, 0xe3]);
        if pending {
            p.bytes.extend([0xdb, 0x7c, 0x24, 48]);
        } // Masked empty-store sets IE.
        p.word(96, 0x037e);
        p.bytes.extend([0xd9, 0x6c, 0x24, 96]);
        p.bytes
            .extend(if pending { [0xd9, 0xe8] } else { [0xd9, 0xc0] });
        let source = image(&p.finish(42), None);
        let (status, fields, text) = run(&source);
        assert_ne!(status, 0, "{text}");
        assert_eq!(fields["code"], "native-dispatch");
        assert_eq!(fields["runtime_failure"], "x87-exception");
        assert!(!fields.contains_key("value"));
        assert_eq!(fields["service_stops"], "0");
    }
    println!(
        "x87_data_refusal_native_available=true x87_data_refusal_native_programs=2 x87_data_refusal_native_returns=0"
    );
}
