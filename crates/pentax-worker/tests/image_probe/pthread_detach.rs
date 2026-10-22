// SPDX-License-Identifier: GPL-2.0-only
//! Original real guest detach/lifecycle programs; no commercial bytes or data.
use super::*;
const MAIN: i32 = 0x1000;
const FIRST: i32 = 0x2000;
const SECOND: i32 = 0x3000;
const DATA: i32 = 0x4400;
// Kernel create/detach, observed POSIX detach, Join, sleep, self and errno.
const IMPORTS: [&str; 7] = [
    "6UgtwV+0zb4",
    "4qGrR6eoP9Y",
    "+U1R4WtXvoc",
    "onNY9Byn-W8",
    "QcteRwbsnV0",
    "aI+OeCz8xrQ",
    "9BcDykPmo1I",
];

struct Code {
    base: i32,
    bytes: Vec<u8>,
    failed: Vec<usize>,
    calls: u64,
}
impl Code {
    fn new(base: i32) -> Self {
        Self {
            base,
            bytes: vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec, 0, 1, 0, 0],
            failed: Vec::new(),
            calls: 0,
        }
    }
    fn reg(&mut self, register: u8, value: u64) {
        self.bytes
            .extend([0x48 | u8::from(register >= 8), 0xb8 + (register & 7)]);
        self.bytes.extend(value.to_le_bytes());
    }
    fn pointer(&mut self, register: u8, address: i32) {
        rip(
            &mut self.bytes,
            self.base,
            &[
                0x48 | if register >= 8 { 4 } else { 0 },
                0x8d,
                ((register & 7) << 3) | 5,
            ],
            address,
        );
    }
    fn call(&mut self, slot: i32) {
        rip(&mut self.bytes, self.base, &[0xff, 0x15], 0xf00 + slot * 8);
        self.calls += 1;
    }
    fn expect_reg(&mut self, register: u8, value: u64) {
        self.reg(10, value);
        self.bytes
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        self.bytes.extend([0x0f, 0x85, 0, 0, 0, 0]);
        self.failed.push(self.bytes.len() - 4);
    }
    fn expect(&mut self, value: u64) {
        self.expect_reg(0, value);
    }
    fn stack(&mut self, opcode: u8, register: u8, offset: u8) {
        self.bytes.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            opcode,
            0x44 | ((register & 7) << 3),
            0x24,
            offset,
        ]);
    }
    fn write(&mut self, address: i32, value: u64) {
        self.reg(0, value);
        self.pointer(11, address);
        self.bytes.extend([0x49, 0x89, 0x03]);
    }
    fn read(&mut self, address: i32) {
        self.pointer(11, address);
        self.bytes.extend([0x49, 0x8b, 0x03]);
    }
    fn word(&mut self, address: i32, value: u64) {
        self.read(address);
        self.expect(value);
    }
    fn fs(&mut self, write: bool) {
        self.bytes
            .extend([0x64, 0x48, if write { 0x89 } else { 0x8b }, 4, 0x25]);
        self.bytes.extend(0x28_u32.to_le_bytes());
    }
    fn tls(&mut self, value: u64) {
        self.fs(false);
        self.expect(value);
    }
    fn create(&mut self, entry: i32, output: i32) {
        self.pointer(7, output);
        self.reg(6, 0);
        self.pointer(2, entry);
        self.reg(1, 0x1234);
        self.pointer(8, NAME);
        self.call(0);
        self.expect(0);
    }
    fn target(&mut self, handle: i32) {
        self.read(handle);
        self.bytes.extend([0x48, 0x89, 0xc7]);
    }
    fn detach(&mut self, handle: i32, slot: i32, expected: u64) {
        self.target(handle);
        self.call(slot);
        self.expect(expected);
    }
    fn join(&mut self, handle: i32, output: i32, expected: u64) {
        self.target(handle);
        self.pointer(6, output);
        self.call(3);
        self.expect(expected);
    }
    fn sleep(&mut self, micros: u64) {
        self.reg(7, micros);
        self.call(4);
        self.expect(0);
    }
    fn seed_errno(&mut self) {
        self.call(6);
        self.stack(0x89, 0, 80);
        self.bytes.extend([0xc7, 0, 0x54, 0x76, 0, 0]);
    }
    fn check_errno(&mut self) {
        self.stack(0x8b, 0, 80);
        self.bytes.extend([0x8b, 0]);
        self.expect(0x7654);
    }
    fn finish(mut self, value: u64) -> (Vec<u8>, u64) {
        self.reg(0, value);
        self.finish_current()
    }
    fn finish_current(mut self) -> (Vec<u8>, u64) {
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x1000);
        (self.bytes, self.calls)
    }
}

fn image(main: &[u8], children: &[(i32, Vec<u8>)]) -> Input {
    let original = scoped_system_image(&[], &IMPORTS, "libkernel", "libkernel", 1);
    let mut bytes = original.bytes.clone();
    // Only slot2 uses the exact observed libScePosix library; all other roles
    // retain their existing kernel tuple. No inferred POSIX cross-product.
    let old_length = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    put(&mut bytes, 0x400 + old_length, b"libScePosix\0");
    put(&mut bytes, 0x218, &((old_length + 12) as u64).to_le_bytes());
    let symbol = 0x600 + 3 * 24;
    let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
    put(&mut bytes, 0x400 + name + 11, b"#E#C");
    for (index, (tag, value)) in [
        (0x6100_0015_i64, old_length as u64 | (1 << 32) | (4 << 48)),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(12_u64 * 16).to_le_bytes());
    }
    bytes.resize(0x14000, 0);
    put(&mut bytes, 24, &(MAIN as u64).to_le_bytes());
    put(&mut bytes, MAIN as usize, main);
    for (address, code) in children {
        assert!((*address == FIRST || *address == SECOND) && code.len() < 0x1000);
        put(&mut bytes, *address as usize, code);
    }
    put(&mut bytes, 96, &0x4000_u64.to_le_bytes());
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (offset, value) in [
        (184, 0x4000_u64),
        (192, 0x4000),
        (200, 0),
        (208, 0x10000),
        (216, 0x10000),
        (224, 0x4000),
    ] {
        put(&mut bytes, offset, &value.to_le_bytes());
    }
    create_input(bytes)
}

fn completed(source: &Input, calls: u64, returns: u64) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["scheduling_returned"], returns.to_string(), "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
}

#[test]
fn native_detach_main_null_unknown_and_repeated_calls_keep_tls_errno_and_unused_output() {
    let mut main = Code::new(MAIN);
    main.seed_errno();
    main.reg(0, 10);
    main.fs(true);
    main.reg(12, 0x1212);
    main.write(DATA + 32, 0xa5);
    for (slot, expected) in [(1, 0x8002_0016), (2, 22)] {
        main.reg(7, 0);
        main.pointer(6, MAIN);
        main.call(slot);
        main.expect(expected);
    }
    for (slot, expected) in [(1, 0x8002_0003), (2, 3)] {
        main.reg(7, u64::MAX);
        main.pointer(6, MAIN);
        main.call(slot);
        main.expect(expected);
    }
    main.call(5);
    main.pointer(11, DATA);
    main.bytes.extend([0x49, 0x89, 0x03]);
    for slot in [2, 1, 2] {
        main.detach(DATA, slot, 0);
    }
    main.word(DATA + 32, 0xa5);
    main.tls(10);
    main.check_errno();
    main.expect_reg(12, 0x1212);
    let (main, calls) = main.finish(42);
    completed(&image(&main, &[]), calls, 0);
    println!(
        "pthread_detach_main_native_available=true pthread_detach_main_native_programs=1 pthread_detach_main_native_errors=4 pthread_detach_main_native_repeats=2"
    );
}

#[test]
fn native_detach_active_completed_and_self_threads_reject_join_before_output_and_really_return() {
    for self_detach in [false, true] {
        let mut main = Code::new(MAIN);
        main.seed_errno();
        main.reg(0, 10);
        main.fs(true);
        main.create(FIRST, DATA);
        main.word(DATA + 8, 1);
        if !self_detach {
            main.detach(DATA, 2, 0);
        }
        main.detach(DATA, 1, 0);
        main.write(DATA + 32, 0xa5);
        main.join(DATA, MAIN, 0x8002_0016); // RX result is never touched for a detached target.
        main.word(DATA + 32, 0xa5);
        main.sleep(100_000);
        main.word(DATA + 8, 2);
        main.detach(DATA, 2, 0); // Successful actual RET retains detached owner identity.
        main.join(DATA, DATA + 32, 0x8002_0016);
        main.word(DATA + 32, 0xa5);
        main.tls(10);
        main.check_errno();
        let (main, calls) = main.finish(42);
        let mut child = Code::new(FIRST);
        child.expect_reg(7, 0x1234);
        child.reg(0, 30);
        child.fs(true);
        if self_detach {
            child.call(5);
            child.bytes.extend([0x48, 0x89, 0xc7]);
            child.call(2);
            child.expect(0);
        }
        child.write(DATA + 8, 1);
        child.sleep(50_000);
        child.tls(30);
        child.write(DATA + 8, 2);
        let (child, child_calls) = child.finish(77);
        completed(&image(&main, &[(FIRST, child)]), calls + child_calls, 1);
    }
    println!(
        "pthread_detach_child_native_available=true pthread_detach_child_native_programs=2 pthread_detach_child_native_returns=2 pthread_detach_child_native_join_refusals=4"
    );
}

#[test]
fn native_detach_preserves_accepted_join_then_reaped_handle_is_not_detachable() {
    let mut main = Code::new(MAIN);
    main.write(DATA + 56, 0);
    main.create(FIRST, DATA);
    main.word(DATA + 8, 1);
    main.create(SECOND, DATA + 16);
    main.word(DATA + 24, 1);
    main.write(DATA + 40, 1);
    main.join(DATA, DATA + 32, 0);
    main.word(DATA + 32, 77);
    main.join(DATA + 16, DATA + 32, 0);
    main.word(DATA + 32, 88);
    main.detach(DATA, 1, 0x8002_0003);
    main.detach(DATA, 2, 3);
    // Report the actual number of yielding polls, not a guessed host delay.
    // Low32 remains the successful original-program result; high32 binds the
    // variable number of mediated calls below to an actual guest counter.
    main.read(DATA + 56);
    main.bytes.extend([0x48, 0xc1, 0xe0, 32]);
    main.reg(10, 42);
    main.bytes.extend([0x4c, 0x09, 0xd0]);
    let (main, calls) = main.finish_current();
    let mut child = Code::new(FIRST);
    child.write(DATA + 8, 1);
    let poll = child.bytes.len();
    child.read(DATA + 24);
    child.reg(10, 2);
    child.bytes.extend([0x4c, 0x39, 0xd0]);
    child.bytes.extend([0x0f, 0x84, 0, 0, 0, 0]);
    let ready = child.bytes.len() - 4;
    child.read(DATA + 56);
    child.reg(10, 64);
    child.bytes.extend([0x4c, 0x39, 0xd0]);
    child.bytes.extend([0x0f, 0x83, 0, 0, 0, 0]);
    child.failed.push(child.bytes.len() - 4);
    child.reg(10, 1);
    child.bytes.extend([0x4c, 0x01, 0xd0]);
    child.pointer(11, DATA + 56);
    child.bytes.extend([0x49, 0x89, 0x03]);
    child.sleep(1); // Suspend FIRST so other runnable contexts can make progress.
    child.bytes.push(0xe9);
    let back = poll as i32 - (child.bytes.len() as i32 + 4);
    child.bytes.extend(back.to_le_bytes());
    let done = child.bytes.len() as i32 - (ready as i32 + 4);
    child.bytes[ready..ready + 4].copy_from_slice(&done.to_le_bytes());
    child.word(DATA + 24, 2);
    let (child, child_calls) = child.finish(77);
    let mut detacher = Code::new(SECOND);
    detacher.write(DATA + 24, 1);
    detacher.sleep(1);
    detacher.word(DATA + 40, 1);
    detacher.detach(DATA, 2, 22);
    detacher.detach(DATA, 1, 0x8002_0016);
    detacher.write(DATA + 24, 2);
    let (detacher, detacher_calls) = detacher.finish(88);
    let source = image(&main, &[(FIRST, child), (SECOND, detacher)]);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    let value = fields["value"].parse::<u64>().unwrap();
    assert_eq!(value as u32, 42, "{text}");
    let polls = value >> 32;
    assert!((1..=64).contains(&polls), "{text}");
    assert_eq!(
        fields["service_stops"],
        (calls + child_calls + detacher_calls - 1 + polls).to_string(),
        "{text}"
    );
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["scheduling_returned"], "2", "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
    println!(
        "pthread_detach_join_native_available=true pthread_detach_join_native_programs=1 pthread_detach_join_native_claim_refusals=2 pthread_detach_join_native_returns=2"
    );
}
