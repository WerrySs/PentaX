// SPDX-License-Identifier: GPL-2.0-only
//! Original real guest programs for non-LIFO fiber entry and continuations.
//! Source callee-saved guarantees are distinct from emulator CPU transport.
use super::*;

const MAIN: i32 = 0x1000;
const FIRST: i32 = 0x2000;
const SECOND: i32 = 0x3000;
const OBJECT_A: i32 = 0x4100;
const OBJECT_B: i32 = 0x4200;
const OUTPUT: i32 = 0x4400;
const CONTEXT_A: i32 = 0x8000;
const CONTEXT_B: i32 = 0xc000;
const STACK_BYTES: u64 = 0x2000;
const INVALID: u64 = 0x8059_0004;
const PERMISSION: u64 = 0x8059_0005;
const STATE: u64 = 0x8059_0006;
// Source-pinned imports; the first five include all actual observed imports.
const IMPORTS: [&str; 6] = [
    "hVYD7Ou2pCQ",
    "a0LLrZWac0M",
    "PFT2S-tJ7Uk",
    "B0ZX2hx9DMw",
    "JeNX5F-NzQU",
    "p+zLIOg27zU",
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
    fn bad(&mut self, opcode: u8) {
        self.bytes.extend([0x0f, opcode, 0, 0, 0, 0]);
        self.failed.push(self.bytes.len() - 4);
    }
    fn expect_reg(&mut self, register: u8, value: u64) {
        self.reg(10, value);
        self.bytes
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        self.bad(0x85);
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
    fn local_pointer(&mut self, register: u8, offset: u8) {
        self.stack(0x8d, register, offset);
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
    fn tls(&mut self, before: u64, after: u64) {
        self.fs(false);
        self.expect(before);
        self.reg(0, after);
        self.fs(true);
    }
    fn initialize(&mut self, object: i32, entry: i32, context: i32, argument: u64) {
        self.pointer(7, object);
        self.pointer(6, NAME);
        self.pointer(2, entry);
        self.reg(1, argument);
        if context == 0 {
            self.reg(8, 0);
            self.reg(9, 0);
        } else {
            self.pointer(8, context);
            self.reg(9, STACK_BYTES);
        }
        self.reg(0, 0);
        self.stack(0x89, 0, 0);
        self.reg(0, 0x0350_0000);
        self.stack(0x89, 0, 8);
        self.call(0);
        self.expect(0);
    }
    fn run(&mut self, object: i32, argument: u64, returned: u64, result: u64) {
        self.pointer(7, object);
        self.reg(6, argument);
        self.pointer(2, OUTPUT);
        self.call(1);
        self.expect(result);
        self.word(OUTPUT, returned);
    }
    fn transfer(&mut self, object: Option<i32>, argument: u64, resumed: u64) {
        if let Some(object) = object {
            self.pointer(7, object);
            self.reg(6, argument);
            self.local_pointer(2, 32);
            self.call(2);
        } else {
            self.reg(7, argument);
            self.local_pointer(6, 32);
            self.call(3);
        }
        self.expect(0);
        self.stack(0x8b, 0, 32);
        self.expect(resumed);
    }
    fn self_is(&mut self, expected: Option<i32>) {
        self.local_pointer(7, 40);
        self.call(5);
        self.expect(0);
        self.stack(0x8b, 0, 40);
        if let Some(object) = expected {
            self.pointer(10, object);
            self.bytes.extend([0x4c, 0x39, 0xd0]);
            self.bad(0x85);
        } else {
            self.expect(0);
        }
    }
    fn stack_is(&mut self, context: i32) {
        self.pointer(10, context + 8);
        self.bytes.extend([0x4c, 0x39, 0xd4]);
        self.bad(0x82); // RSP below admitted stack.
        self.pointer(10, context + STACK_BYTES as i32);
        self.bytes.extend([0x4c, 0x39, 0xd4]);
        self.bad(0x83);
        self.stack(0x89, 4, 48); // Exact stack pointer, retained in its own stack.
    }
    fn same_stack(&mut self) {
        self.stack(0x8b, 0, 48);
        self.bytes.extend([0x48, 0x39, 0xc4]);
        self.bad(0x85);
    }
    fn vectors(&mut self, store: bool, address: i32) {
        for register in 0..16 {
            rip(
                &mut self.bytes,
                self.base,
                &[
                    0xc5,
                    if register < 8 { 0xfc } else { 0x7c },
                    if store { 0x11 } else { 0x10 },
                    5 | ((register & 7) << 3),
                ],
                address + i32::from(register) * 32,
            );
        }
    }
    fn compare_vector_bytes(&mut self, actual: i32, expected: i32) {
        self.pointer(7, actual);
        self.pointer(6, expected);
        self.reg(1, 64);
        let begin = self.bytes.len();
        self.bytes.extend([0x48, 0x8b, 0x07, 0x48, 0x3b, 0x06]);
        self.bad(0x85);
        self.bytes.extend([
            0x48, 0x83, 0xc7, 8, 0x48, 0x83, 0xc6, 8, 0x48, 0xff, 0xc9, 0x0f, 0x85,
        ]);
        self.bytes
            .extend((begin as i32 - (self.bytes.len() as i32 + 4)).to_le_bytes());
    }
    fn controls(&mut self, mxcsr: u32, x87: u16) {
        self.reg(0, u64::from(mxcsr));
        self.stack(0x89, 0, 64);
        self.bytes.extend([0x0f, 0xae, 0x54, 0x24, 64]);
        self.reg(0, u64::from(x87));
        self.stack(0x89, 0, 72);
        self.bytes.extend([0xd9, 0x6c, 0x24, 72]);
    }
    fn check_controls(&mut self, mxcsr: u32, x87: u16) {
        self.bytes
            .extend([0x0f, 0xae, 0x5c, 0x24, 80, 0x8b, 0x44, 0x24, 80]);
        self.expect(u64::from(mxcsr));
        self.bytes
            .extend([0xd9, 0x7c, 0x24, 88, 0x0f, 0xb7, 0x44, 0x24, 88]);
        self.expect(u64::from(x87));
    }
    fn finish(mut self, value: u64) -> (Vec<u8>, u64) {
        self.reg(0, value);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x1000);
        (self.bytes, self.calls)
    }
}

fn image(
    main: &[u8],
    fibers: &[(i32, Vec<u8>)],
    module: &str,
    library: &str,
    version: u64,
) -> Input {
    image_with_extra(main, fibers, module, library, version, &[], "libkernel")
}

#[allow(clippy::too_many_arguments)]
fn image_with_extra(
    main: &[u8],
    fibers: &[(i32, Vec<u8>)],
    module: &str,
    library: &str,
    version: u64,
    extra: &[&str],
    extra_library: &str,
) -> Input {
    let mut imports = IMPORTS.to_vec();
    imports.extend_from_slice(extra);
    let original = scoped_system_image(&[], &imports, module, library, version);
    let mut bytes = original.bytes.clone();
    // Only observed execution roles have the paired libSceFiber alias. GetSelf
    // remains in its existing canonical Fiber scope; never invent an alias.
    let old_length = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let mut names = b"Fiber\0Fiber\0".to_vec();
    let mut descriptors = vec![
        (
            0x6100_000f_i64,
            old_length as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x6100_0015, (old_length as u64 + 6) | (1 << 32) | (5 << 48)),
    ];
    let symbol = 0x600 + 6 * 24;
    let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
    put(&mut bytes, 0x400 + name + 11, b"#F#E");
    if !extra.is_empty() {
        let module = old_length + names.len();
        names.extend(format!("libkernel\0{extra_library}\0").bytes());
        descriptors.extend([
            (
                0x6100_000f_i64,
                module as u64 | (1 << 32) | (1 << 40) | (6 << 48),
            ),
            (0x6100_0015, (module as u64 + 10) | (1 << 32) | (7 << 48)),
        ]);
        for index in 6..imports.len() {
            let symbol = 0x600 + (index + 1) * 24;
            let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
            put(&mut bytes, 0x400 + name + 11, b"#H#G");
        }
    }
    put(&mut bytes, 0x400 + old_length, &names);
    put(
        &mut bytes,
        0x218,
        &((old_length + names.len()) as u64).to_le_bytes(),
    );
    descriptors.push((0, 0));
    let count = 10 + descriptors.len();
    for (index, (tag, value)) in descriptors.into_iter().enumerate() {
        put(&mut bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &((count as u64) * 16).to_le_bytes());
    }
    bytes.resize(0x14000, 0);
    put(&mut bytes, 24, &(MAIN as u64).to_le_bytes());
    put(&mut bytes, MAIN as usize, main);
    for (address, code) in fibers {
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

fn assert_completed(source: &Input, calls: u64) {
    assert_completed_with_initializers(source, calls, 0);
}

fn assert_completed_with_initializers(source: &Input, calls: u64, initializers: u64) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(
        fields["initializer_calls"],
        initializers.to_string(),
        "{text}"
    );
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn native_fiber_run_yield_resume_and_entry_return_use_actual_arguments_stacks_and_tls() {
    for context in [CONTEXT_A, 0] {
        let mut main = Code::new(MAIN);
        main.initialize(OBJECT_A, FIRST, context, 0x1234);
        main.self_is(None);
        main.reg(0, 10);
        main.fs(true);
        main.reg(3, 0x1111);
        main.reg(12, 0x1212);
        main.reg(13, 0x1313);
        main.run(OBJECT_A, 11, 101, 0);
        main.expect_reg(3, 0x1111);
        main.expect_reg(12, 0x1212);
        main.expect_reg(13, 0x1313);
        main.tls(12, 20);
        main.self_is(None);
        main.run(OBJECT_A, 22, 202, 0);
        main.tls(22, 30);
        main.run(OBJECT_A, 33, 0, STATE);
        main.tls(32, 40);
        main.run(OBJECT_A, 44, 0, STATE);
        main.pointer(7, OBJECT_A);
        main.call(4);
        main.expect(STATE);
        let (main, main_calls) = main.finish(42);

        let mut entry = Code::new(FIRST);
        entry.expect_reg(7, 0x1234);
        entry.expect_reg(6, 11);
        if context != 0 {
            entry.stack_is(context);
        }
        entry.self_is(Some(OBJECT_A));
        entry.tls(10, 12);
        entry.reg(3, 0xaaaa);
        entry.reg(12, 0xbbbb);
        entry.reg(13, 0xcccc);
        entry.write(OUTPUT + 8, 1); // Retired once before the first yield.
        entry.transfer(None, 101, 22);
        if context != 0 {
            entry.same_stack();
        }
        entry.expect_reg(3, 0xaaaa);
        entry.expect_reg(12, 0xbbbb);
        entry.expect_reg(13, 0xcccc);
        entry.word(OUTPUT + 8, 1);
        entry.write(OUTPUT + 8, 2);
        entry.tls(20, 22);
        entry.transfer(None, 202, 33);
        if context != 0 {
            entry.same_stack();
        }
        entry.word(OUTPUT + 8, 2);
        entry.write(OUTPUT + 8, 3);
        entry.tls(30, 32);
        let (entry, entry_calls) = entry.finish(u64::MAX);
        let source = image(&main, &[(FIRST, entry)], "libSceFiber", "libSceFiber", 1);
        assert_completed(&source, main_calls + entry_calls);
    }
    println!(
        "fiber_run_native_available=true fiber_run_native_programs=2 fiber_run_native_yields=4 fiber_run_native_resumes=4"
    );
}

#[test]
fn native_fiber_switches_resume_non_lifo_and_return_the_current_fibers_value() {
    let mut main = Code::new(MAIN);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 0x111);
    main.initialize(OBJECT_B, SECOND, CONTEXT_B, 0x222);
    main.reg(0, 1);
    main.fs(true);
    main.run(OBJECT_A, 11, 101, 0);
    main.tls(3, 4);
    main.run(OBJECT_A, 22, 202, 0);
    main.tls(5, 6);
    main.run(OBJECT_B, 33, 303, 0);
    main.tls(8, 9);
    main.run(OBJECT_B, 55, 0, STATE);
    main.tls(10, 11);
    main.run(OBJECT_A, 66, 0, STATE);
    main.tls(12, 13);
    let (main, calls) = main.finish(42);
    let mut a = Code::new(FIRST);
    a.expect_reg(7, 0x111);
    a.expect_reg(6, 11);
    a.stack_is(CONTEXT_A);
    a.self_is(Some(OBJECT_A));
    a.tls(1, 2);
    a.reg(12, 0xa1);
    a.transfer(Some(OBJECT_B), 12, 22);
    a.same_stack();
    a.expect_reg(12, 0xa1);
    a.self_is(Some(OBJECT_A));
    a.tls(4, 5);
    a.transfer(None, 202, 44);
    a.same_stack();
    a.tls(7, 8);
    a.transfer(None, 303, 66);
    a.same_stack();
    a.tls(11, 12);
    let (a, a_calls) = a.finish(0);
    let mut b = Code::new(SECOND);
    b.expect_reg(7, 0x222);
    b.expect_reg(6, 12);
    b.stack_is(CONTEXT_B);
    b.self_is(Some(OBJECT_B));
    b.tls(2, 3);
    b.reg(12, 0xb1);
    b.transfer(None, 101, 33);
    b.same_stack();
    b.expect_reg(12, 0xb1);
    b.tls(6, 7);
    b.transfer(Some(OBJECT_A), 44, 55);
    b.same_stack();
    b.tls(9, 10);
    let (b, b_calls) = b.finish(0);
    let source = image(
        &main,
        &[(FIRST, a), (SECOND, b)],
        "libSceFiber",
        "libSceFiber",
        1,
    );
    assert_completed(&source, calls + a_calls + b_calls);
    println!(
        "fiber_switch_native_available=true fiber_switch_native_programs=1 fiber_switch_native_transfers=2 fiber_switch_native_thread_returns=3"
    );
}

#[test]
fn native_idle_fiber_migrates_between_real_pthreads_and_uses_each_current_run_root_and_tls() {
    let mut main = Code::new(MAIN);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 0x123);
    main.reg(0, 10);
    main.fs(true);
    main.run(OBJECT_A, 11, 111, 0);
    main.tls(12, 20);
    main.pointer(7, OUTPUT + 24);
    main.reg(6, 0);
    main.pointer(2, SECOND);
    main.reg(1, 0);
    main.pointer(8, NAME);
    main.call(6);
    main.expect(0);
    main.read(OUTPUT + 24);
    main.bytes.extend([0x48, 0x89, 0xc7]);
    main.pointer(6, OUTPUT + 32);
    main.call(7);
    main.expect(0);
    main.word(OUTPUT + 32, 77);
    main.tls(20, 20);
    main.run(OBJECT_A, 33, 0, STATE);
    main.tls(22, 22);
    main.word(OUTPUT + 8, 3);
    let (main, calls) = main.finish(42);

    let mut fiber = Code::new(FIRST);
    fiber.expect_reg(7, 0x123);
    fiber.expect_reg(6, 11);
    fiber.stack_is(CONTEXT_A);
    fiber.self_is(Some(OBJECT_A));
    fiber.tls(10, 12);
    fiber.reg(12, 0xa1);
    fiber.write(OUTPUT + 8, 1);
    fiber.transfer(None, 111, 22);
    fiber.same_stack();
    fiber.expect_reg(12, 0xa1);
    fiber.self_is(Some(OBJECT_A));
    fiber.tls(30, 32);
    fiber.word(OUTPUT + 8, 1);
    fiber.write(OUTPUT + 8, 2);
    fiber.transfer(None, 222, 33);
    fiber.same_stack();
    fiber.expect_reg(12, 0xa1);
    fiber.self_is(Some(OBJECT_A));
    fiber.tls(20, 22);
    fiber.word(OUTPUT + 8, 2);
    fiber.write(OUTPUT + 8, 3);
    let (fiber, fiber_calls) = fiber.finish(0);

    let mut child = Code::new(SECOND);
    child.reg(0, 30);
    child.fs(true);
    child.self_is(None);
    child.run(OBJECT_A, 22, 222, 0);
    child.self_is(None);
    child.tls(32, 40);
    let (child, child_calls) = child.finish(77);
    let source = image_with_extra(
        &main,
        &[(FIRST, fiber), (SECOND, child)],
        "libSceFiber",
        "libSceFiber",
        1,
        &["6UgtwV+0zb4", "onNY9Byn-W8"],
        "libkernel",
    );
    assert_completed(&source, calls + fiber_calls + child_calls);
    println!(
        "fiber_migration_native_available=true fiber_migration_native_programs=1 fiber_migration_native_pthreads=2 fiber_migration_native_resumes=2"
    );
}

#[test]
fn native_fiber_invalid_and_running_targets_keep_source_errors_without_entry_or_output() {
    let mut main = Code::new(MAIN);
    main.reg(7, 0);
    main.reg(6, 0);
    main.reg(2, u64::MAX);
    main.call(1);
    main.expect(INVALID);
    main.reg(7, 0);
    main.call(2);
    main.expect(INVALID);
    main.reg(7, 77);
    main.reg(6, u64::MAX);
    main.call(3);
    main.expect(PERMISSION);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 17);
    main.write(OUTPUT, 0xa5);
    main.pointer(7, OBJECT_A);
    main.reg(6, 11);
    main.pointer(2, OUTPUT);
    main.call(2);
    main.expect(PERMISSION);
    main.word(OUTPUT, 0xa5);
    main.run(OBJECT_A, 11, 99, 0);
    main.pointer(7, OBJECT_A);
    main.call(4);
    main.expect(0);
    main.write(OUTPUT, 0xa5);
    main.pointer(7, OBJECT_A);
    main.reg(6, 0);
    main.pointer(2, OUTPUT);
    main.call(1);
    main.expect(STATE);
    main.word(OUTPUT, 0xa5);
    let (main, calls) = main.finish(42);
    let mut entry = Code::new(FIRST);
    entry.expect_reg(7, 17);
    entry.expect_reg(6, 11);
    entry.pointer(7, OBJECT_A);
    entry.reg(6, 0);
    entry.reg(2, u64::MAX);
    entry.call(1);
    entry.expect(PERMISSION);
    entry.pointer(7, OBJECT_A);
    entry.call(2);
    entry.expect(STATE);
    entry.pointer(7, OBJECT_A);
    entry.call(4);
    entry.expect(STATE);
    // The suspended fiber never resumes after Finalize; this service occurs
    // once and the callback suffix must not retire or claim a native return.
    entry.reg(7, 99);
    entry.reg(6, 0);
    entry.call(3);
    let (entry, entry_calls) = entry.finish(0);
    let source = image(&main, &[(FIRST, entry)], "libSceFiber", "libSceFiber", 1);
    assert_completed(&source, calls + entry_calls);
    println!(
        "fiber_errors_native_available=true fiber_errors_native_programs=1 fiber_errors_native_cases=8"
    );
}

#[test]
fn native_finalized_fiber_reinitialization_cannot_reuse_a_parked_entry_or_continuation() {
    let mut main = Code::new(MAIN);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 1);
    main.run(OBJECT_A, 11, 111, 0);
    main.pointer(7, OBJECT_A);
    main.call(4);
    main.expect(0);
    main.initialize(OBJECT_A, SECOND, CONTEXT_A, 2);
    main.run(OBJECT_A, 22, 0, STATE);
    main.word(OUTPUT + 8, 2);
    let (main, calls) = main.finish(42);
    let mut first = Code::new(FIRST);
    first.expect_reg(7, 1);
    first.expect_reg(6, 11);
    first.write(OUTPUT + 8, 1);
    first.reg(7, 111);
    first.reg(6, 0);
    first.call(3);
    first.bytes.extend([0x0f, 0x0b]); // Reusing this saved continuation must fail.
    let (first, first_calls) = first.finish(0);
    let mut second = Code::new(SECOND);
    second.expect_reg(7, 2);
    second.expect_reg(6, 22);
    second.write(OUTPUT + 8, 2);
    let (second, second_calls) = second.finish(0);
    let source = image(
        &main,
        &[(FIRST, first), (SECOND, second)],
        "libSceFiber",
        "libSceFiber",
        1,
    );
    assert_completed(&source, calls + first_calls + second_calls);
    println!(
        "fiber_reinitialize_native_available=true fiber_reinitialize_native_programs=1 fiber_reinitialize_native_generations=2"
    );
}

#[test]
fn native_fiber_owned_cpu_transport_keeps_all_vector_lanes_control_and_root_flags() {
    // This is the explicit PentaX complete-context transport policy. The pinned
    // source's 80-byte GPR save does not guarantee caller-saved XMM/YMM state.
    let root_data = 0x5000;
    let fiber_data = 0x5400;
    let observed = 0x5800;
    let mut main = Code::new(MAIN);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 1);
    main.controls(0x1f84, 0x127f);
    main.vectors(false, root_data);
    main.pointer(7, OBJECT_A);
    main.reg(6, 11);
    main.pointer(2, OUTPUT);
    main.bytes.push(0xf9); // STC immediately before the admitted Run call.
    main.call(1);
    main.stack(0x89, 0, 8);
    main.bytes.extend([0x9c, 0x58]);
    main.stack(0x89, 0, 16);
    main.stack(0x8b, 0, 8);
    main.expect(0);
    main.word(OUTPUT, 111);
    main.stack(0x8b, 0, 16);
    main.bytes.extend([0x48, 0x83, 0xe0, 1]);
    main.expect(1);
    main.vectors(true, observed);
    main.compare_vector_bytes(observed, root_data);
    main.check_controls(0x1f84, 0x127f);
    main.run(OBJECT_A, 22, 0, STATE);
    main.vectors(true, observed);
    main.compare_vector_bytes(observed, root_data);
    main.check_controls(0x1f84, 0x127f);
    let (main, calls) = main.finish(42);
    let mut fiber = Code::new(FIRST);
    fiber.controls(0x3f80, 0x167f);
    fiber.vectors(false, fiber_data);
    fiber.transfer(None, 111, 22);
    fiber.vectors(true, observed);
    fiber.compare_vector_bytes(observed, fiber_data);
    fiber.check_controls(0x3f80, 0x167f);
    let (fiber, fiber_calls) = fiber.finish(0);
    let original = image(&main, &[(FIRST, fiber)], "libSceFiber", "libSceFiber", 1);
    let mut bytes = original.bytes.clone();
    for (address, seed) in [
        (root_data, 0x1122_3344_5566_0000_u64),
        (fiber_data, 0xaabb_ccdd_eeff_0000_u64),
    ] {
        for lane in 0..64 {
            put(
                &mut bytes,
                address as usize + lane * 8,
                &(seed + lane as u64).to_le_bytes(),
            );
        }
    }
    assert_completed(&create_input(bytes), calls + fiber_calls);
    println!(
        "fiber_cpu_native_available=true fiber_cpu_native_programs=1 fiber_cpu_native_vector_registers=16 fiber_cpu_native_vector_bytes=512"
    );
}

#[test]
fn native_fiber_yield_inside_once_callback_restores_both_suspended_call_chains() {
    let mut main = Code::new(MAIN);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 1);
    main.reg(0, 10);
    main.fs(true);
    main.run(OBJECT_A, 11, 111, 0);
    main.tls(12, 20);
    main.run(OBJECT_A, 22, 222, 0);
    main.tls(24, 30);
    main.run(OBJECT_A, 33, 0, STATE);
    main.tls(32, 40);
    main.word(OUTPUT + 8, 3);
    let (main, calls) = main.finish(42);

    let mut fiber = Code::new(FIRST);
    fiber.self_is(Some(OBJECT_A));
    fiber.tls(10, 10);
    fiber.pointer(7, OUTPUT + 16);
    fiber.pointer(6, SECOND);
    fiber.call(6);
    fiber.expect(0);
    fiber.word(OUTPUT + 16, 1);
    fiber.word(OUTPUT + 8, 2);
    fiber.tls(22, 24);
    // A completed once call cannot re-enter the yielded callback's prefix.
    fiber.pointer(7, OUTPUT + 16);
    fiber.pointer(6, SECOND);
    fiber.call(6);
    fiber.expect(0);
    fiber.transfer(None, 222, 33);
    fiber.self_is(Some(OBJECT_A));
    fiber.tls(30, 32);
    fiber.write(OUTPUT + 8, 3);
    let (fiber, fiber_calls) = fiber.finish(0);

    let mut callback = Code::new(SECOND);
    callback.self_is(Some(OBJECT_A));
    callback.tls(10, 12);
    callback.word(OUTPUT + 8, 0);
    callback.write(OUTPUT + 8, 1);
    callback.transfer(None, 111, 22);
    callback.self_is(Some(OBJECT_A));
    callback.tls(20, 22);
    callback.word(OUTPUT + 8, 1);
    callback.write(OUTPUT + 8, 2);
    let (callback, callback_calls) = callback.finish(u64::MAX); // void callback result.
    let source = image_with_extra(
        &main,
        &[(FIRST, fiber), (SECOND, callback)],
        "libSceFiber",
        "libSceFiber",
        1,
        &["Z4QosVuAsA0"],
        "Posix",
    );
    assert_completed_with_initializers(&source, calls + fiber_calls + callback_calls, 1);
    println!(
        "fiber_callback_native_available=true fiber_callback_native_programs=1 fiber_callback_native_yields=2 fiber_callback_native_initializers=1"
    );
}

#[test]
fn native_fiber_faulted_entries_and_import_scope_errors_do_not_publish_a_return() {
    let mut main = Code::new(MAIN);
    main.initialize(OBJECT_A, FIRST, CONTEXT_A, 1);
    main.pointer(7, OBJECT_A);
    main.reg(6, 11);
    main.pointer(2, OUTPUT);
    main.call(1);
    let (main, calls) = main.finish(42);
    let source = image(
        &main,
        &[(FIRST, vec![0x0f, 0x0b])],
        "libSceFiber",
        "libSceFiber",
        1,
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    for (module, library, version, expected) in [
        ("libkernel", "libkernel", 1, "unavailable-function"),
        ("libSceFiber", "libSceFiber", 2, "import-version-mismatch"),
    ] {
        let mut main = Code::new(MAIN);
        main.reg(7, 0);
        main.reg(6, 0);
        main.reg(2, 0);
        main.call(1);
        let (main, _) = main.finish(42);
        let source = image(&main, &[], module, library, version);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], expected, "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
    println!(
        "fiber_refusal_native_available=true fiber_refusal_native_programs=3 fiber_refusal_native_cases=3"
    );
}
