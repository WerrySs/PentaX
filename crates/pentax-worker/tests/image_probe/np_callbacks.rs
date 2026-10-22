// SPDX-License-Identifier: GPL-2.0-only
//! Original native NP callback consumers. Guest branches require real callback
//! writes, exact arguments, snapshot order, nested returns and thread continuity.
//! Sealed foreign-owner/source/replay receipts are exercised by core tests.
use super::*;

const ENTRY: i32 = 0x1000;
const FIRST: i32 = 0x6000;
const SECOND: i32 = 0x8000;
const CHILD: i32 = 0xa000;
const DATA: i32 = 0x10000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const MXCSR: u32 = 0x1f84;
const GPRS: [u8; 12] = [1, 2, 3, 6, 7, 8, 9, 11, 12, 13, 14, 15];
const CHECK: &str = "3Zl8BePTh9Y";
const REGISTER: &str = "qQJfO8HAiaY";
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
        self.word(8, u64::from(MXCSR | (((self.base as u32 >> 12) & 3) << 13)));
        self.bytes.extend([0x0f, 0xae, 0x54, 0x24, 8]);
        for register in 0..16 {
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.word(112 + i as u8 * 8, value ^ ((self.base as u64) << 32));
            }
            self.vector(true, register);
        }
        for register in GPRS {
            self.reg(register, gpr_pattern(register) ^ self.base as u64);
        }
    }
    fn check_context(&mut self, environment_base: i32) {
        // A real same-thread callback may change MXCSR; caller GPR/YMM state
        // is restored independently of the continuing thread FP environment.
        for register in GPRS {
            if matches!(register, 1 | 11) {
                continue;
            }
            self.expect(register, gpr_pattern(register) ^ self.base as u64);
        }
        for register in 0..16 {
            self.vector(false, register);
            for (i, value) in vector_pattern(register).into_iter().enumerate() {
                self.check(112 + i as u8 * 8, value ^ ((self.base as u64) << 32));
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
        let expected_mxcsr = MXCSR | (((environment_base as u32 >> 12) & 3) << 13);
        self.check(88, (CANARY & !0xffff_ffff) | u64::from(expected_mxcsr));
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

impl Code {
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
    fn put(&mut self, address: i32, value: u64) {
        self.path(10, address);
        self.reg(0, value);
        self.bytes.extend([0x49, 0x89, 0x02]);
    }
    fn record(&mut self, address: i32, register: u8) {
        self.path(10, address);
        self.bytes.extend([
            0x49 | if register >= 8 { 4 } else { 0 },
            0x89,
            ((register & 7) << 3) | 2,
        ]);
    }
    fn at(&mut self, address: i32, value: u64) {
        self.path(0, address);
        self.bytes.extend([0x48, 0x8b, 0]);
        self.expect(0, value);
    }
    fn pointer(&mut self, register: u8, target: i32) {
        self.path(10, target);
        self.bytes
            .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
        self.bad();
    }
    fn register(&mut self, entry: i32, userdata: i32, ordinal: u64) {
        self.path(7, entry);
        self.path(6, userdata);
        self.call(1);
        self.expect(0, ordinal);
    }
    fn check_pending(&mut self) {
        self.call(0);
        self.expect(0, 0);
    }
    fn errno(&mut self, record: i32) {
        self.call(6);
        self.record(record + 32, 0);
        self.bytes.extend([0xc7, 0, 0x35, 0, 0, 0]);
    }
    fn arguments(&mut self, record: i32) {
        self.expect(7, 1); // Default UserConfig.id, not a host identity.
        self.expect(6, 1); // Offline source state.
        self.pointer(2, record);
        self.record(record + 8, 7);
        self.record(record + 16, 6);
        self.record(record + 24, 2);
        self.path(10, record);
        self.bytes.extend([0x49, 0xff, 0x02]);
        self.call(6);
        self.path(10, record + 32);
        self.bytes.extend([0x4d, 0x8b, 0x12, 0x4c, 0x39, 0xd0]);
        self.bad(); // Callback keeps the current thread's actual errno object.
        self.bytes.extend([0x83, 0x38, 0x35]);
        self.bad();
        self.call(2);
        self.expect(0, 1); // Genuine nested ordinary service, not a callback-return shortcut.
    }
    fn flags(&mut self) {
        self.bytes.extend([0x48, 0x39, 0xc0, 0xf9]); // CMP RAX,RAX; STC.
    }
    fn flags_unchanged(&mut self) {
        for condition in [0x83, 0x85, 0x8b, 0x88, 0x80] {
            self.bad_if(condition);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Scope {
    Canonical,
    ObservedCheck,
    WrongModule,
    WrongLibraryVersion,
    WrongModuleVersion,
    UnobservedRegisterAlias,
}
fn image(main: &[u8], callbacks: &[&[u8]], child: Option<&[u8]>, scope: Scope) -> Input {
    let kernel = [
        pentax_kernel::Service::Getpid,
        pentax_kernel::Service::PthreadCreate,
        pentax_kernel::Service::PthreadJoin,
        pentax_kernel::Service::PthreadYield,
        pentax_kernel::Service::ErrnoLocation,
    ];
    let mut imports = vec![
        (CHECK, 0),
        (
            REGISTER,
            if matches!(scope, Scope::UnobservedRegisterAlias) {
                0
            } else {
                1
            },
        ),
    ];
    for service in kernel {
        let binding = pentax_kernel::Service::registrations()
            .iter()
            .find(|entry| entry.service == service)
            .unwrap();
        imports.push((binding.nid, 2));
    }
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x14000, 0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x10000_u64.to_le_bytes());
    put(&mut bytes, 104, &0x10000_u64.to_le_bytes());
    bytes[0x400..0x1000].fill(0);
    assert!(main.len() < 0x4000);
    put(&mut bytes, ENTRY as usize, main);
    for (index, callback) in callbacks.iter().enumerate() {
        assert!(index < 2 && callback.len() < 0x2000);
        put(
            &mut bytes,
            (FIRST + index as i32 * 0x2000) as usize,
            callback,
        );
    }
    if let Some(child) = child {
        assert!(child.len() < 0x4000);
        put(&mut bytes, CHILD as usize, child);
    }
    // Independent RW data keeps callbacks source-backed RX and records writable.
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x10000_u64),
        (192, 0x10000),
        (200, 0),
        (208, 0x4000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    let check_names = match scope {
        Scope::Canonical => ("NpManager", "NpManager"),
        Scope::WrongModule => ("NpManagerOther", "NpManager"),
        _ => ("libSceNpManager", "libSceNpManager"),
    };
    let mut names = vec![0];
    let mut scopes = Vec::new();
    for (module, library) in [
        check_names,
        ("NpManager", "NpManager"),
        ("libkernel", "libkernel"),
    ] {
        let module_offset = names.len() as u64;
        names.extend_from_slice(module.as_bytes());
        names.push(0);
        let library_offset = names.len() as u64;
        names.extend_from_slice(library.as_bytes());
        names.push(0);
        scopes.push((module_offset, library_offset));
    }
    for (index, (nid, scope)) in imports.iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        let module = char::from(b'C' + *scope as u8 * 2);
        let library = char::from(b'D' + *scope as u8 * 2);
        names.extend_from_slice(format!("{nid}#{library}#{module}\0").as_bytes());
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
        (39, (imports.len() as u64 + 1) * 24),
        (11, 24),
    ];
    for (index, (module, library)) in scopes.into_iter().enumerate() {
        let id = 2 + index as u64 * 2;
        let major = if index == 0 && matches!(scope, Scope::WrongModuleVersion) {
            2
        } else {
            1
        };
        let library_version = if index == 0 && matches!(scope, Scope::WrongLibraryVersion) {
            2
        } else {
            1
        };
        dynamic.push((0x6100_000f, module | (1 << 32) | (major << 40) | (id << 48)));
        dynamic.push((
            0x6100_0015,
            library | (library_version << 32) | ((id + 1) << 48),
        ));
    }
    dynamic.extend([(7, 0x800), (8, imports.len() as u64 * 24), (9, 24), (0, 0)]);
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
fn returned(source: &Input) {
    let (status, fields, text) = run(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
}
fn record(p: &mut Code, address: i32, count: u64) {
    p.at(address, count);
    p.at(address + 8, 1);
    p.at(address + 16, 1);
    p.path(0, address + 24);
    p.bytes.extend([0x48, 0x8b, 0]);
    p.pointer(0, address);
}

impl Code {
    fn hold_x87(&mut self, significand: u64) {
        self.bytes.extend([0xdb, 0xe3]);
        self.word(16, significand);
        self.word(24, 0x3fff);
        self.bytes.extend([0xdb, 0x6c, 0x24, 16]);
    }
    fn check_x87(&mut self, significand: u64, exponent: u16) {
        self.word(48, CANARY);
        self.word(56, CANARY);
        self.bytes.extend([0xdb, 0x7c, 0x24, 48]);
        self.check(48, significand);
        self.check(56, (CANARY & !0xffff) | u64::from(exponent));
        self.word(64, CANARY);
        self.bytes.extend([0xdd, 0x7c, 0x24, 64]);
        self.check(64, CANARY & !0xffff);
    }
}
fn snapshot(scope: Scope) -> Input {
    let mut main = Code::new(ENTRY);
    main.check_pending(); // No registrations and no callback execution.
    main.reg(7, 0);
    main.reg(6, u64::MAX);
    main.call(1);
    main.expect(0, 0x8055_0003);
    main.errno(DATA);
    main.errno(DATA + 0x100);
    main.register(FIRST, DATA, 1);
    main.register(FIRST, DATA + 0x100, 0x8055_0008); // Function alone is the duplicate key.
    main.register(SECOND, DATA + 0x100, 2);
    main.seed_context();
    main.hold_x87(0x8000_0000_0000_0007);
    main.flags();
    main.call(0);
    main.flags_unchanged();
    main.expect(0, 0);
    main.check_context(SECOND);
    main.check_x87(0, 0); // Last callback deliberately leaves its exact FLDZ value.
    record(&mut main, DATA, 1);
    record(&mut main, DATA + 0x100, 1);
    main.check_pending();
    main.at(DATA, 1);
    main.at(DATA + 0x100, 1);
    let mut first = Code::new(FIRST);
    first.arguments(DATA);
    first.at(DATA + 0x100, 0); // Row order is visible through actual writes.
    first.check_pending(); // All old pending rows were cleared BEFORE this call.
    first.at(DATA, 1);
    first.at(DATA + 0x100, 0);
    first.bytes.extend([0xdb, 0xe3, 0xd9, 0xee]); // Deliberately changes the guest x87 stack.
    first.seed_context();
    let mut second = Code::new(SECOND);
    second.arguments(DATA + 0x100);
    second.at(DATA, 1);
    second.bytes.extend([0xdb, 0xe3, 0xd9, 0xee]);
    second.seed_context();
    image(
        &main.finish(42),
        &[&first.finish(u64::MAX), &second.finish(0x1234)],
        None,
        scope,
    )
}
fn registration_during_callback(reentrant: bool) -> Input {
    let mut main = Code::new(ENTRY);
    main.errno(DATA);
    main.errno(DATA + 0x100);
    main.register(FIRST, DATA, 1);
    main.check_pending();
    main.at(DATA, 1);
    main.at(DATA + 0x100, u64::from(reentrant));
    main.at(DATA + 48, 1);
    main.check_pending(); // Deferred callback is delivered only by this future check.
    record(&mut main, DATA + 0x100, 1);
    main.check_pending();
    main.at(DATA + 0x100, 1);
    let mut first = Code::new(FIRST);
    first.arguments(DATA);
    first.check_pending(); // Empty recursive snapshot cannot replay itself.
    first.register(SECOND, DATA + 0x100, 2);
    first.at(DATA + 0x100, 0);
    if reentrant {
        first.check_pending(); // Only the appended row belongs to this new check.
        first.at(DATA + 0x100, 1);
    }
    first.put(DATA + 48, 1);
    let mut second = Code::new(SECOND);
    second.arguments(DATA + 0x100);
    second.at(DATA, 1);
    second.at(DATA + 48, u64::from(!reentrant));
    image(
        &main.finish(42),
        &[&first.finish(u64::MAX), &second.finish(u64::MAX)],
        None,
        Scope::ObservedCheck,
    )
}
#[test]
fn native_np_callbacks_execute_exact_arguments_void_results_and_clear_snapshot_before_invocation() {
    for scope in [Scope::Canonical, Scope::ObservedCheck] {
        returned(&snapshot(scope));
    }
    println!("np_callback_native_snapshot_programs=2 np_callback_native_snapshot_returns=2");
}
#[test]
fn native_np_callbacks_defer_appended_rows_and_allow_actual_nested_new_snapshots() {
    for reentrant in [false, true] {
        returned(&registration_during_callback(reentrant));
    }
    println!("np_callback_native_nested_programs=2 np_callback_native_nested_returns=2");
}

fn threaded_snapshots() -> Input {
    let mut main = Code::new(ENTRY);
    main.errno(DATA);
    main.register(FIRST, DATA, 1);
    main.stack(0x8d, 7, 208);
    main.reg(6, 0);
    main.path(2, CHILD);
    main.reg(1, 0);
    main.reg(8, 0);
    main.call(3);
    main.expect(0, 0);
    main.seed_context();
    main.hold_x87(0x8000_0000_0000_0009);
    main.call(0);
    main.expect(0, 0);
    main.check_context(FIRST);
    main.check_x87(0x8000_0000_0000_000b, 0x3fff);
    main.at(DATA + 64, 3);
    main.stack(0x8b, 7, 208);
    main.stack(0x8d, 6, 216);
    main.call(4);
    main.expect(0, 0);
    main.check(216, 77);
    record(&mut main, DATA, 1);
    record(&mut main, DATA + 0x100, 1);
    main.at(DATA + 64, 4);
    main.check_pending();
    let mut first = Code::new(FIRST);
    first.arguments(DATA);
    first.seed_context();
    first.hold_x87(0x8000_0000_0000_000b);
    first.put(DATA + 64, 1);
    first.flags();
    first.call(5); // Force a real owned guest-thread handoff; source return is void.
    first.flags_unchanged();
    first.check_context(FIRST);
    first.check_x87(0x8000_0000_0000_000b, 0x3fff);
    first.at(DATA + 64, 2); // Other thread is suspended inside its own callback.
    first.check_pending(); // Must not replace either outstanding callback frame.
    // Re-establish the value already consumed by the isolation check.
    first.hold_x87(0x8000_0000_0000_000b);
    first.put(DATA + 64, 3);
    let mut second = Code::new(SECOND);
    second.arguments(DATA + 0x100);
    second.seed_context();
    second.hold_x87(0x8000_0000_0000_000d);
    second.put(DATA + 64, 2);
    second.flags();
    second.call(5);
    second.flags_unchanged();
    second.check_context(SECOND);
    second.check_x87(0x8000_0000_0000_000d, 0x3fff);
    second.at(DATA + 64, 3);
    second.hold_x87(0x8000_0000_0000_000d);
    second.put(DATA + 64, 4);
    let mut child = Code::new(CHILD);
    // At most eight explicit handoffs. No host sleeps or timing expectation.
    let mut ready = Vec::new();
    for _ in 0..8 {
        child.path(0, DATA + 64);
        child.bytes.extend([0x48, 0x83, 0x38, 1, 0x0f, 0x84]);
        ready.push(child.bytes.len());
        child.bytes.extend([0; 4]);
        child.call(5);
    }
    child.bytes.extend([0x0f, 0x0b]);
    let destination = child.bytes.len();
    for branch in ready {
        child.bytes[branch..branch + 4]
            .copy_from_slice(&((destination - branch - 4) as i32).to_le_bytes());
    }
    child.errno(DATA + 0x100);
    child.path(10, DATA + 32);
    child.bytes.extend([0x4d, 0x8b, 0x12, 0x4c, 0x39, 0xd0]);
    child.bad_if(0x84); // Independent errno objects, not merely equal contents.
    child.register(SECOND, DATA + 0x100, 2);
    child.seed_context();
    child.hold_x87(0x8000_0000_0000_000f);
    child.check_pending();
    child.check_context(SECOND);
    child.check_x87(0x8000_0000_0000_000d, 0x3fff);
    child.at(DATA + 64, 4);
    image(
        &main.finish(42),
        &[&first.finish(u64::MAX), &second.finish(u64::MAX)],
        Some(&child.finish(77)),
        Scope::ObservedCheck,
    )
}
#[test]
fn native_np_callbacks_keep_two_live_frames_and_thread_local_full_cpu_context_through_guest_handoffs()
 {
    let source = threaded_snapshots();
    let (status, fields, text) = run(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["scheduling_threads"], "2", "{text}");
    assert_eq!(fields["scheduling_returned"], "1", "{text}");
    println!("np_callback_native_thread_programs=1 np_callback_native_thread_returns=1");
}

#[test]
fn native_np_callbacks_refuse_unbacked_writable_nonreturning_prefixed_and_wrong_scope_calls() {
    let mut count = 0;
    for target in [None, Some(DATA)] {
        let mut main = Code::new(ENTRY);
        if let Some(target) = target {
            main.path(7, target);
        } else {
            main.reg(7, u64::MAX);
        }
        main.reg(6, 0);
        main.call(1);
        main.expect(0, 1); // Registration is not target-execution authority.
        main.check_pending();
        let source = image(&main.finish(42), &[], None, Scope::ObservedCheck);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "target-class={target:?}: {text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], "provider-validation", "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        count += 1;
    }
    for (body, expected) in [
        (&[0x0f, 0x0b][..], "unsupported-instruction"),
        (&[0xf0, 0xc3][..], "block-translation"),
    ] {
        let mut main = Code::new(ENTRY);
        main.register(FIRST, DATA, 1);
        main.check_pending();
        let source = image(&main.finish(42), &[body], None, Scope::ObservedCheck);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "{expected}: {text}");
        assert_eq!(fields["code"], expected, "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        count += 1;
    }
    for scope in [
        Scope::WrongModule,
        Scope::WrongLibraryVersion,
        Scope::WrongModuleVersion,
        Scope::UnobservedRegisterAlias,
    ] {
        let mut main = Code::new(ENTRY);
        if matches!(scope, Scope::UnobservedRegisterAlias) {
            main.register(FIRST, DATA, 1);
        } else {
            main.check_pending();
        }
        let source = image(&main.finish(42), &[&[0xc3]], None, scope);
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "scope={scope:?}: {text}");
        if matches!(
            scope,
            Scope::WrongLibraryVersion | Scope::WrongModuleVersion
        ) {
            // The linker rejects a known identity with incompatible versions
            // before entry. This is stronger than a deferred unavailable use.
            assert_eq!(fields["code"], "import-version-mismatch", "{text}");
            assert_eq!(fields["phase"], "admission", "{text}");
            assert_eq!(fields["native_instructions"], "0", "{text}");
            assert_eq!(fields["service_stops"], "0", "{text}");
            assert_eq!(fields["unavailable_functions"], "unknown", "{text}");
        } else {
            assert_eq!(fields["code"], "unavailable-function", "{text}");
            assert_eq!(fields["phase"], "main-entry", "{text}");
            assert_eq!(fields["unavailable_functions"], "1", "{text}");
            assert_eq!(fields["service_stops"], "1", "{text}");
        }
        assert!(!fields.contains_key("value"), "{text}");
        count += 1;
    }
    assert_eq!(count, 8);
    println!("np_callback_native_refusal_programs={count} np_callback_native_refusals={count}");
}
