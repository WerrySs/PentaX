// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded yield consumers: real child/parent turns without polling.
use super::*;

const CHILD: i32 = 0x1100;
const IMPORTS: [&str; 6] = [
    "6UgtwV+0zb4", // create
    "T72hz6ffq08", // observed source-void yield
    "aI+OeCz8xrQ", // self
    "9BcDykPmo1I", // errno
    "EI-5-jlq2dE", // thread ID
    "onNY9Byn-W8", // join
];

struct Code {
    base: i32,
    p: Program,
}
impl Code {
    fn new(base: i32) -> Self {
        Self {
            base,
            p: Program::new(),
        }
    }
    fn call(&mut self, slot: i32) {
        rip(&mut self.p.code, self.base, &[0xff, 0x15], 0xf00 + slot * 8);
        self.p.calls += 1;
    }
    fn vector(&mut self, value: u64) {
        self.p.reg(0, value);
        self.p.code.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0]); // MOVQ XMM0,RAX
    }
    fn check_vector(&mut self, value: u64) {
        self.p.code.extend([0x66, 0x48, 0x0f, 0x7e, 0xc0]); // MOVQ RAX,XMM0
        self.p.expect(value);
    }
    fn fs(&mut self, write: bool) {
        self.p
            .code
            .extend([0x64, 0x48, if write { 0x89 } else { 0x8b }, 4, 0x25]);
        self.p.code.extend(0x28_u32.to_le_bytes());
    }
    fn tls(&mut self, value: u64) {
        self.fs(false);
        self.p.expect(value);
    }
    fn compare(&mut self, register: u8, slot: u8, equal: bool) {
        self.p.load(10, slot);
        self.p
            .code
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        // For a required difference, make an equality branch the failing edge.
        self.p
            .code
            .extend([0x0f, if equal { 0x85 } else { 0x84 }, 0, 0, 0, 0]);
        self.p.branches.push(self.p.code.len() - 4);
    }
    fn check_register(&mut self, register: u8, value: u64) {
        self.p.reg(10, value);
        self.p
            .code
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        checked(&mut self.p.code, &mut self.p.branches);
    }
    fn yield_now(&mut self) {
        // Poison every unused argument, including pointers. The source takes
        // none and has no integer result to assert or turn into errno.
        for register in [7, 6, 2, 1, 8, 9] {
            self.p.reg(register, u64::MAX);
        }
        self.call(1);
    }
    fn errno(&mut self, value: u8) {
        self.p.load(0, 0);
        self.p.code.extend([0x8b, 0]);
        self.p.expect(value.into());
    }
    fn finish(self) -> (Vec<u8>, u64) {
        self.p.finish(None)
    }
}

fn input(main: &[u8], child: Option<&[u8]>) -> Input {
    let original = image(&[], &IMPORTS);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x1800, 0);
    put(&mut bytes, 0x800, main);
    if let Some(child) = child {
        assert!(CHILD as usize + child.len() < 0x1800);
        put(&mut bytes, CHILD as usize, child);
    }
    put(&mut bytes, 96, &0x1800_u64.to_le_bytes());
    create_input(bytes)
}

fn success(source: &Input, calls: u64, returned: u64) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(
        fields["scheduling_returned"],
        returned.to_string(),
        "{text}"
    );
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}

#[test]
fn native_yield_executes_peer_before_next_instruction_and_restores_each_owned_context() {
    let mut main = Code::new(0x800);
    main.call(3);
    main.p.save(0);
    main.p.code.extend([0xc7, 0, 37, 0, 0, 0]);
    main.call(2);
    main.p.save(8);
    main.p.reg(0, 0x1111);
    main.fs(true);
    main.p.reg(12, 0x1212);
    main.vector(0x1111_2222_3333_4444);
    main.p.set(64, 0); // Shared stage, followed by child identity observations.
    main.p.ptr(7, 16);
    main.p.reg(6, 0);
    main.p.path(2, CHILD);
    main.p.ptr(1, 64);
    main.p.reg(8, 0);
    main.call(0);
    main.p.expect(0);
    // Create forces the child's first turn. The child's yield must hand back
    // before it checks stage 2; a success stub reaches the child's failure.
    main.p.word(64, 1);
    main.check_register(12, 0x1212);
    main.check_vector(0x1111_2222_3333_4444);
    main.tls(0x1111);
    main.errno(37);
    main.call(2);
    main.compare(0, 8, true);
    main.compare(0, 72, false);
    main.call(3);
    main.compare(0, 0, true);
    main.compare(0, 88, false);
    main.call(4);
    main.p.expect(1);
    main.p.word(80, 2);
    main.p.load(0, 16);
    main.compare(0, 72, true); // Published handle belongs to the executed child.
    main.p.set(64, 2);
    main.yield_now();
    // Far below every priority quantum, this requires an explicit turn.
    main.p.word(64, 3);
    main.check_register(12, 0x1212);
    main.check_vector(0x1111_2222_3333_4444);
    main.tls(0x1111);
    main.errno(37);
    let (main, main_calls) = main.finish();

    let mut child = Code::new(CHILD);
    child.p.code.extend([0x48, 0x89, 0xfb]); // RBX = parent's shared stage pointer.
    child.call(2);
    child.p.save(8);
    child.p.code.extend([0x48, 0x89, 0x43, 8]);
    child.call(4);
    child.p.expect(2);
    child.p.code.extend([0x48, 0x89, 0x43, 16]);
    child.call(3);
    child.p.save(0);
    child
        .p
        .code
        .extend([0x48, 0x89, 0x43, 24, 0xc7, 0, 19, 0, 0, 0]);
    child.p.reg(0, 0x2222);
    child.fs(true);
    child.p.reg(12, 0x3434);
    child.vector(0x5555_6666_7777_8888);
    child.p.code.extend([0x48, 0xc7, 3, 1, 0, 0, 0]);
    child.yield_now();
    child.p.code.extend([0x48, 0x8b, 3]);
    child.p.expect(2);
    child.check_register(12, 0x3434);
    child.check_vector(0x5555_6666_7777_8888);
    child.tls(0x2222);
    child.errno(19);
    child.call(2);
    child.compare(0, 8, true);
    child.call(3);
    child.compare(0, 0, true);
    child.call(4);
    child.p.expect(2);
    child.p.code.extend([0x48, 0xc7, 3, 3, 0, 0, 0]);
    let (child, child_calls) = child.finish();
    let source = input(&main, Some(&child));
    success(&source, main_calls + child_calls, 1);
}

#[test]
fn native_yield_with_no_eligible_peer_continues_without_guest_wait_or_errno_change() {
    let mut main = Code::new(0x800);
    main.call(3);
    main.p.save(0);
    main.p.code.extend([0xc7, 0, 37, 0, 0, 0]);
    main.p.reg(12, 0x5656);
    main.vector(0x9999_aaaa_bbbb_cccc);
    for _ in 0..3 {
        main.yield_now();
        main.check_register(12, 0x5656);
        main.check_vector(0x9999_aaaa_bbbb_cccc);
        main.errno(37);
    }
    let (main, calls) = main.finish();
    success(&input(&main, None), calls, 0);
}

#[test]
fn native_yield_skips_a_parked_joiner_without_satisfying_its_wait() {
    let mut main = Code::new(0x800);
    main.p.set(64, 0);
    main.p.ptr(7, 16);
    main.p.reg(6, 0);
    main.p.path(2, CHILD);
    main.p.ptr(1, 64);
    main.p.reg(8, 0);
    main.call(0);
    main.p.expect(0);
    main.p.word(64, 1);
    main.p.set(64, 2);
    main.p.load(7, 16);
    main.p.ptr(6, 24);
    main.call(5); // Parks the parent until the actual child return.
    main.p.expect(0);
    main.p.word(24, 42);
    main.p.word(64, 3);
    let (main, main_calls) = main.finish();

    let mut child = Code::new(CHILD);
    child
        .p
        .code
        .extend([0x48, 0x89, 0xfb, 0x48, 0xc7, 3, 1, 0, 0, 0]);
    child.yield_now();
    child.p.code.extend([0x48, 0x8b, 3]);
    child.p.expect(2);
    child.yield_now(); // Only the parent exists, and its join is still pending.
    child.p.code.extend([0x48, 0xc7, 3, 3, 0, 0, 0]);
    let (child, child_calls) = child.finish();
    success(&input(&main, Some(&child)), main_calls + child_calls, 1);
}
