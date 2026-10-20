// SPDX-License-Identifier: GPL-2.0-only
//! Original created-thread cleanup consumers; no process atexit emulation.

use super::*;

const DATA: i32 = 0x4000;
const CHILD: i32 = 0x1100;
const GENERAL: i32 = 0x1500;
const KEY: i32 = 0x1900;
const TRAP: i32 = 0x1d00;

struct Function {
    base: i32,
    code: Vec<u8>,
    branches: Vec<usize>,
    calls: u64,
}

impl Function {
    fn new(base: i32) -> Self {
        let mut value = Self {
            base,
            code: vec![0x53],
            branches: Vec::new(),
            calls: 0,
        };
        value.address(3, DATA); // Preserve RBX; all records belong to this input.
        value
    }
    fn reg(&mut self, reg: u8, value: u64) {
        self.code
            .extend([0x48 | u8::from(reg >= 8), 0xb8 + (reg & 7)]);
        self.code.extend(value.to_le_bytes());
    }
    fn address(&mut self, reg: u8, address: i32) {
        rip(
            &mut self.code,
            self.base,
            &[
                0x48 | if reg >= 8 { 4 } else { 0 },
                0x8d,
                ((reg & 7) << 3) | 5,
            ],
            address,
        );
    }
    fn load(&mut self, reg: u8, offset: u8) {
        assert!(offset < 128);
        self.code.extend([
            0x48 | if reg >= 8 { 4 } else { 0 },
            0x8b,
            0x43 | ((reg & 7) << 3),
            offset,
        ]);
    }
    fn save(&mut self, offset: u8) {
        assert!(offset < 128);
        self.code.extend([0x48, 0x89, 0x43, offset]);
    }
    fn set(&mut self, offset: u8, value: u8) {
        assert!(offset < 128);
        self.code.extend([0x48, 0xc7, 0x43, offset, value, 0, 0, 0]);
    }
    fn call(&mut self, slot: i32) {
        rip(&mut self.code, self.base, &[0xff, 0x15], 0xf00 + slot * 8);
        self.calls += 1;
    }
    fn expect(&mut self, value: u64) {
        self.reg(10, value);
        self.code.extend([0x4c, 0x39, 0xd0]);
        checked(&mut self.code, &mut self.branches);
    }
    fn word(&mut self, offset: u8, value: u64) {
        self.load(0, offset);
        self.expect(value);
    }
    fn pointer(&mut self, reg: u8, address: i32) {
        self.address(10, address);
        self.code
            .extend([0x4c | u8::from(reg >= 8), 0x39, 0xd0 | (reg & 7)]);
        checked(&mut self.code, &mut self.branches);
    }
    fn wait_release(&mut self) {
        let start = self.code.len();
        self.code.extend([0x48, 0x83, 0x7b, 120, 1, 0x0f, 0x85]);
        self.code
            .extend((start as i32 - (self.code.len() as i32 + 4)).to_le_bytes());
    }
    fn finish(mut self, value: u64) -> Vec<u8> {
        self.reg(0, value);
        self.code.extend([0x5b, 0xc3]);
        finish_checks(&mut self.code, self.branches);
        self.code
    }
}

fn fs(code: &mut Vec<u8>, store: bool) {
    code.extend([0x64, 0x48, if store { 0x89 } else { 0x8b }, 4, 0x25]);
    code.extend(0x28_u32.to_le_bytes());
}

fn image(main: &[u8], imports: &[&str], functions: &[(i32, Vec<u8>)]) -> Input {
    assert!(imports.len() <= 8 && main.len() < 0x700);
    let original = scoped_system_image(main, imports, "libkernel", "libkernel", 1);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x8000, 0);
    put(&mut bytes, 96, &0x2000_u64.to_le_bytes());
    put(&mut bytes, 0x800, main); // Restore the unused filesystem-name window.
    let mut end = 0x1000;
    for (entry, code) in functions {
        assert!(end <= *entry as usize);
        end = *entry as usize + code.len();
        assert!(end <= 0x2000);
        put(&mut bytes, *entry as usize, code);
    }
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x4000_u64),
        (192, 0x4000),
        (200, 0),
        (208, 0x4000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    create_input(bytes)
}

fn store(p: &mut Program, offset: i32, reg: u8) {
    p.path(11, DATA + offset);
    p.code.extend([
        0x49 | if reg >= 8 { 4 } else { 0 },
        0x89,
        3 | ((reg & 7) << 3),
    ]);
}
fn word(p: &mut Program, offset: i32, expected: u64) {
    p.path(11, DATA + offset);
    p.code.extend([0x49, 0x8b, 3]);
    p.expect(expected);
}
fn same(p: &mut Program, left: i32, right: i32, equal: bool) {
    p.path(11, DATA + left);
    p.code.extend([0x49, 0x8b, 3]);
    p.path(11, DATA + right);
    p.code.extend([
        0x4d,
        0x8b,
        0x13,
        0x4c,
        0x39,
        0xd0,
        0x0f,
        if equal { 0x85 } else { 0x84 },
        0,
        0,
        0,
        0,
    ]);
    p.branches.push(p.code.len() - 4);
}
fn start(p: &mut Program, create: i32) {
    p.path(7, DATA);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.path(1, DATA);
    p.reg(8, 0);
    p.call(create);
    p.expect(0);
    p.reg(0, 1);
    store(p, 120, 0);
}
fn wait(p: &mut Program, offset: i32, value: u8) {
    p.path(11, DATA + offset);
    let start = p.code.len();
    p.code
        .extend([0x49, 0x8b, 3, 0x48, 0x83, 0xf8, value, 0x0f, 0x85]);
    p.code
        .extend((start as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
    // Let the completed callback return and the cleanup owner finish. The
    // parent must never leave after observing only a pre-return write.
    p.reg(12, 3000);
    let quantum = p.code.len();
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]);
    p.code
        .extend((quantum as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
}
fn finish(mut p: Program) -> (Vec<u8>, u64) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700);
    (p.code, p.calls)
}
fn success(input: &Input, calls: u64) {
    let (status, fields, text) = probe(input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(
        fields["kernel_handles_live"], fields["kernel_handles_accounted"],
        "{text}"
    );
}

fn identity(f: &mut Function, errno: u8) {
    f.call(0);
    f.load(10, 40);
    f.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut f.code, &mut f.branches);
    f.call(2);
    f.expect(2);
    f.call(1);
    f.load(10, 48);
    f.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut f.code, &mut f.branches);
    f.code.extend([0x8b, 0]);
    f.expect(u64::from(errno));
}

#[test]
fn native_general_then_key_destructor_keep_child_identity_tls_errno_and_restore_parent() {
    let imports = [
        "aI+OeCz8xrQ",
        "9BcDykPmo1I",
        "EI-5-jlq2dE",
        "6UgtwV+0zb4",
        "rNhWz+lvOMU",
        "geDaqgH9lTg",
        "+BzXYkqYeLE",
        "eoht7mQOCmo",
    ];
    let mut p = Program::new();
    p.call(0);
    store(&mut p, 24, 0);
    p.call(1);
    store(&mut p, 32, 0);
    p.code.extend([0xc7, 0, 37, 0, 0, 0]);
    p.call(2);
    p.expect(1);
    p.reg(0, 13);
    fs(&mut p.code, true);
    p.path(7, GENERAL);
    p.call(4);
    p.path(7, DATA + 8);
    p.path(6, KEY);
    p.call(5);
    p.expect(0);
    start(&mut p, 3);
    wait(&mut p, 16, 2);
    word(&mut p, 16, 2);
    word(&mut p, 56, 2);
    same(&mut p, 0, 40, true);
    same(&mut p, 24, 40, false);
    same(&mut p, 32, 48, false);
    p.call(0);
    p.path(11, DATA + 24);
    p.code.extend([0x4d, 0x8b, 0x13, 0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(2);
    p.expect(1);
    p.call(1);
    p.path(11, DATA + 32);
    p.code.extend([0x4d, 0x8b, 0x13, 0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x8b, 0]);
    p.expect(37);
    p.path(11, DATA + 48);
    p.code.extend([0x49, 0x8b, 3, 0x8b, 0]);
    p.expect(21);
    fs(&mut p.code, false);
    p.expect(13);
    let (main, parent_calls) = finish(p);

    let mut child = Function::new(CHILD);
    child.wait_release();
    for (slot, offset) in [(0, 40), (1, 48), (2, 56)] {
        child.call(slot);
        child.save(offset);
        if slot == 1 {
            child.code.extend([0xc7, 0, 19, 0, 0, 0]);
        }
    }
    child.reg(0, 29);
    fs(&mut child.code, true);
    child.load(7, 8);
    child.address(6, DATA + 64);
    child.call(6);
    child.expect(0);
    let child_calls = child.calls;

    let mut general = Function::new(GENERAL);
    general.word(16, 0);
    identity(&mut general, 19);
    fs(&mut general.code, false);
    general.expect(29);
    general.load(0, 48);
    general.code.extend([0xc7, 0, 20, 0, 0, 0]);
    general.reg(0, 30);
    fs(&mut general.code, true);
    general.set(16, 1);
    let general_calls = general.calls;

    let mut key = Function::new(KEY);
    key.pointer(7, DATA + 64);
    key.word(16, 1);
    key.load(7, 8);
    key.call(7);
    key.expect(0); // Cleared before this destructor, rather than after return.
    identity(&mut key, 20);
    fs(&mut key.code, false);
    key.expect(30);
    key.load(0, 48);
    key.code.extend([0xc7, 0, 21, 0, 0, 0]);
    key.reg(0, 31);
    fs(&mut key.code, true);
    key.set(16, 2);
    let key_calls = key.calls;
    let source = image(
        &main,
        &imports,
        &[
            (CHILD, child.finish(73)),
            (GENERAL, general.finish(u64::MAX)),
            (KEY, key.finish(u64::MAX)),
        ],
    );
    success(
        &source,
        parent_calls + child_calls + general_calls + key_calls,
    );
}

#[test]
fn native_key_destructor_reads_cleared_slot_and_requeues_exactly_four_passes() {
    let imports = ["6UgtwV+0zb4", "geDaqgH9lTg", "+BzXYkqYeLE", "eoht7mQOCmo"];
    let mut p = Program::new();
    p.path(7, DATA + 8);
    p.path(6, KEY);
    p.call(1);
    p.expect(0);
    start(&mut p, 0);
    wait(&mut p, 16, 4);
    word(&mut p, 16, 4);
    p.reg(7, 0);
    p.call(3);
    p.expect(0); // Parent values never receive the child's final requeue.
    let (main, calls) = finish(p);
    let mut child = Function::new(CHILD);
    child.wait_release();
    child.load(7, 8);
    child.address(6, DATA);
    child.call(2);
    child.expect(0);
    let mut key = Function::new(KEY);
    key.pointer(7, DATA);
    key.load(7, 8);
    key.call(3);
    key.expect(0);
    key.load(0, 16);
    key.code.extend([
        0x48, 0x83, 0xc0, 1, 0x48, 0x83, 0xf8, 4, 0x0f, 0x87, 0, 0, 0, 0,
    ]);
    key.branches.push(key.code.len() - 4); // A fifth real callback must fault.
    key.save(16);
    key.load(7, 8);
    key.address(6, DATA);
    key.call(2); // Requeue even on pass4; cleanup must discard that remainder.
    key.expect(0);
    let total = calls + child.calls + 4 * key.calls;
    let source = image(
        &main,
        &imports,
        &[(CHILD, child.finish(73)), (KEY, key.finish(u64::MAX))],
    );
    success(&source, total);
}

#[test]
fn native_key_cleanup_scans_live_later_slots_and_skips_deleted_generation_values() {
    let imports = [
        "6UgtwV+0zb4",
        "geDaqgH9lTg",
        "PrdHuuDekhY",
        "+BzXYkqYeLE",
        "eoht7mQOCmo",
    ];
    let mut p = Program::new();
    for (offset, target) in [(8, GENERAL), (16, TRAP)] {
        p.path(7, DATA + offset);
        p.path(6, target);
        p.call(1);
        p.expect(0);
    }
    start(&mut p, 0);
    wait(&mut p, 48, 2);
    word(&mut p, 32, 1);
    word(&mut p, 40, 1);
    word(&mut p, 24, 1); // Lowest free slot reused, with a new generation.
    let (main, calls) = finish(p);
    let mut child = Function::new(CHILD);
    child.wait_release();
    for (offset, value) in [(8, DATA + 64), (16, DATA + 80)] {
        child.load(7, offset);
        child.address(6, value);
        child.call(3);
        child.expect(0);
    }
    let mut first = Function::new(GENERAL);
    first.pointer(7, DATA + 64);
    first.load(7, 16);
    first.call(2);
    first.expect(0);
    first.address(7, DATA + 24);
    first.address(6, KEY);
    first.call(1);
    first.expect(0);
    first.load(7, 24);
    first.call(4);
    first.expect(0); // The stale nonnull old-generation value is not inherited.
    first.load(7, 24);
    first.address(6, DATA + 72);
    first.call(3);
    first.expect(0);
    first.set(32, 1);
    let mut later = Function::new(KEY);
    later.pointer(7, DATA + 72);
    later.word(32, 1);
    later.load(7, 24);
    later.call(4);
    later.expect(0);
    later.set(40, 1);
    later.set(48, 2);
    let total = calls + child.calls + first.calls + later.calls;
    let source = image(
        &main,
        &imports,
        &[
            (CHILD, child.finish(73)),
            (GENERAL, first.finish(u64::MAX)),
            (KEY, later.finish(u64::MAX)),
            (TRAP, vec![0x0f, 0x0b]),
        ],
    );
    success(&source, total);
}

#[test]
fn native_count_report_are_opaque_but_real_cleanup_unbacked_and_faulting_targets_refuse() {
    let imports = [
        "6UgtwV+0zb4",
        "pB-yGZ2nQ9o",
        "WhCc1w3EhSI",
        "rNhWz+lvOMU",
        "geDaqgH9lTg",
        "+BzXYkqYeLE",
    ];
    // NULL general cleanup is the positive control. The other two cases have
    // identical keys and poison count/report registrations, but a real bad
    // callback must stop before that key can publish its completion marker.
    for target in [None, Some(u64::MAX), Some(TRAP as u64)] {
        let mut p = Program::new();
        p.reg(7, u64::MAX);
        p.call(1);
        p.reg(7, u64::MAX - 1);
        p.call(2);
        match target {
            Some(value) if value == u64::MAX => p.reg(7, value),
            Some(_) => p.path(7, TRAP),
            None => p.reg(7, 0),
        }
        p.call(3);
        p.path(7, DATA + 8);
        p.path(6, KEY);
        p.call(4);
        p.expect(0);
        start(&mut p, 0);
        wait(&mut p, 16, 1);
        word(&mut p, 16, 1);
        let (main, calls) = finish(p);
        let mut child = Function::new(CHILD);
        child.wait_release();
        child.load(7, 8);
        child.address(6, DATA);
        child.call(5);
        child.expect(0);
        let total = calls + child.calls;
        let mut key = Function::new(KEY);
        key.pointer(7, DATA);
        key.set(16, 1);
        let source = image(
            &main,
            &imports,
            &[
                (CHILD, child.finish(73)),
                (KEY, key.finish(u64::MAX)),
                (TRAP, vec![0x0f, 0x0b]),
            ],
        );
        if target.is_none() {
            success(&source, total);
        } else {
            let (status, fields, text) = probe(&source);
            assert_eq!(status, 2, "{text}");
            assert_eq!(
                fields["code"],
                if target == Some(u64::MAX) {
                    "runtime-service"
                } else {
                    "unsupported-instruction"
                },
                "{text}"
            );
            assert_eq!(fields["phase"], "thread-entry", "{text}");
            assert_eq!(fields["initializer_calls"], "0", "{text}");
            assert_eq!(fields["service_stops"], total.to_string(), "{text}");
            assert!(!fields.contains_key("value"), "{text}");
        }
    }
}
