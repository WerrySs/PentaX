// SPDX-License-Identifier: GPL-2.0-only
//! Original initializers run through the real nested-call/return mechanism.
use super::*;
const INIT: i32 = 0xc00;
fn fs(p: &mut Program, store: bool, offset: u32) {
    p.code
        .extend([0x64, 0x48, if store { 0x89 } else { 0x8b }, 4, 0x25]);
    p.code.extend(offset.to_le_bytes());
}
fn finish_callback(mut p: Program) -> Vec<u8> {
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x100);
    p.code
}
fn image_with_callbacks(code: &[u8], callbacks: &[(i32, Vec<u8>)]) -> Input {
    let original = posix::scoped_image(code, &[("Z4QosVuAsA0", true), ("k+AXqu2-eBc", false)], 1);
    attach_callbacks(&original, code.len(), callbacks)
}
fn attach_callbacks(original: &Input, code_len: usize, callbacks: &[(i32, Vec<u8>)]) -> Input {
    let mut bytes = original.bytes.clone();
    let mut end = 0x800 + code_len;
    for (entry, callback) in callbacks {
        assert!(end <= *entry as usize);
        end = *entry as usize + callback.len();
        assert!(end <= NAME as usize);
        put(&mut bytes, *entry as usize, callback);
    }
    create_input(bytes)
}

#[test]
fn native_once_return_survives_a_real_nested_module_load_and_source_epoch_append() {
    let mut p = Program::new();
    p.set(8, 0);
    p.reg(0, 13);
    fs(&mut p, true, 0x28);
    p.ptr(7, 8);
    p.path(6, INIT);
    p.call(0);
    p.expect(0);
    p.word(8, 1);
    p.ptr(7, 8);
    p.path(6, INIT);
    p.call(0);
    p.expect(0);
    fs(&mut p, false, 0x28);
    p.expect(42);
    p.save(32);
    let (code, calls) = p.finish(Some(32));
    let mut init = Program::new();
    rip(&mut init.code, INIT, &[0x48, 0x8d, 0x3d], NAME);
    init.reg(6, 17);
    init.reg(2, 0xdead);
    init.reg(1, 0);
    init.reg(8, 0);
    init.ptr(9, 8);
    rip(&mut init.code, INIT, &[0xff, 0x15], 0xf08);
    init.expect(2);
    init.word(8, 7);
    fs(&mut init, false, 0x28);
    init.expect(14);
    init.code.extend([0x48, 0x83, 0xc0, 28]);
    fs(&mut init, true, 0x28);
    let original = posix::scoped_image(&code, &[("Z4QosVuAsA0", true), ("wzvqT4UqKX8", false)], 1);
    let attached = attach_callbacks(&original, code.len(), &[(INIT, finish_callback(init))]);
    let mut bytes = attached.bytes.clone();
    put(
        &mut bytes,
        NAME as usize,
        b"/app0/sce_module/pentax-original.sprx\0",
    );
    let main = create_input(bytes);
    let library = retain_preload(true);
    let (status, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], (calls + 1).to_string());
    assert_eq!(fields["initializer_calls"], "2");
    assert_eq!(fields["phase"], "main-entry");
}

#[test]
fn native_once_return_cannot_publish_done_after_the_initializer_makes_its_control_readonly() {
    let mut p = Program::new();
    p.set(8, 0);
    p.ptr(7, 8);
    p.reg(6, 0x4000);
    p.reg(2, 3);
    p.reg(1, 0);
    p.call(1);
    p.expect(0);
    p.load(0, 8);
    fs(&mut p, true, 0x30);
    p.load(7, 8);
    p.path(6, INIT);
    p.call(0);
    let (code, calls) = p.finish(None);
    let mut init = Program::new();
    fs(&mut init, false, 0x30);
    init.code.extend([0x48, 0x89, 0xc7, 0x8b, 0]);
    init.expect(2);
    init.reg(6, 0x4000);
    init.reg(2, 1);
    rip(&mut init.code, INIT, &[0xff, 0x15], 0xf10);
    init.expect(0);
    let original = posix::scoped_image(
        &code,
        &[
            ("Z4QosVuAsA0", true),
            ("IWIBBdTHit4", false),
            ("vSMAm3cxYTY", false),
        ],
        1,
    );
    let main = attach_callbacks(&original, code.len(), &[(INIT, finish_callback(init))]);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service");
    assert_eq!(fields["service_stops"], (calls + 1).to_string());
    assert_eq!(fields["phase"], "initializer");
    assert_eq!(fields["initializer_calls"], "0");
    assert!(!fields.contains_key("value"));
}
fn check(main: &Input, calls: u64, initializers: u64, fp_status: u32) {
    let (status, fields, text) = probe(main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["initializer_calls"], initializers.to_string());
    assert_eq!(fields["phase"], "main-entry");
    assert_eq!(fields["fp_status"], fp_status.to_string());
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn native_once_observes_in_progress_runs_real_tls_and_services_then_publishes_done_only_once() {
    let mut p = Program::new();
    p.reg(0, 0x1122_3344_0000_007f);
    p.save(8);
    p.set(16, 77);
    p.ptr(0, 8);
    fs(&mut p, true, 0x30);
    p.reg(0, 13);
    fs(&mut p, true, 0x28);
    p.reg(7, 0);
    p.path(6, INIT);
    p.call(0);
    p.expect(22);
    p.ptr(7, 8);
    p.reg(6, 0);
    p.call(0);
    p.expect(22);
    p.ptr(7, 8);
    p.reg(6, u64::MAX);
    p.call(0);
    p.expect(22);
    p.reg(0, 0x1122_3344_0000_0000);
    p.save(8);
    p.ptr(7, 8);
    p.path(6, INIT);
    p.call(0);
    p.expect(0);
    p.word(8, 0x1122_3344_0000_0001);
    p.word(16, 77);
    // DONE does not dereference or call a nonnull invalid initializer address.
    p.ptr(7, 8);
    p.reg(6, u64::MAX);
    p.call(0);
    p.expect(0);
    p.set(48, 0);
    p.code.extend([0xd9, 0x7c, 0x24, 48]); // Initializer's x87 environment survives return.
    p.word(48, 0x127f);
    fs(&mut p, false, 0x28);
    p.expect(42);
    p.save(32);
    let (code, calls) = p.finish(Some(32));
    let mut init = Program::new();
    fs(&mut init, false, 0x30);
    init.code.extend([0x8b, 0]);
    init.expect(2);
    // Real masked division by zero raises guest MXCSR.ZE. This thread state
    // must survive the return from the initializer to its suspended caller.
    init.reg(0, 0x127f);
    init.save(48);
    init.code.extend([0xd9, 0x6c, 0x24, 48]);
    init.code.extend([
        0xb8, 0, 0, 0x80, 0x3f, 0x66, 0x0f, 0x6e, 0xc0, 0x66, 0x0f, 0xef, 0xc9, 0xf3, 0x0f, 0x5e,
        0xc1,
    ]);
    fs(&mut init, false, 0x28);
    init.code.extend([0x48, 0x83, 0xc0, 29]);
    fs(&mut init, true, 0x28);
    rip(&mut init.code, INIT, &[0xff, 0x15], 0xf08);
    init.expect(0x4000);
    init.reg(0, u64::MAX); // void result must not become a negative module error.
    let main = image_with_callbacks(&code, &[(INIT, finish_callback(init))]);
    check(&main, calls + 1, 1, 4);
}

#[test]
fn native_nested_once_initializers_share_tls_and_preserve_each_suspended_caller() {
    let first = 0xb00;
    let mut p = Program::new();
    p.set(8, 0);
    p.set(16, 0);
    p.ptr(0, 16);
    fs(&mut p, true, 0x38);
    p.ptr(7, 8);
    p.path(6, first);
    p.call(0);
    p.expect(0);
    p.word(8, 1);
    p.word(16, 1);
    p.ptr(7, 8);
    p.path(6, first);
    p.call(0);
    p.expect(0);
    fs(&mut p, false, 0x28);
    p.expect(42);
    p.save(32);
    let (code, calls) = p.finish(Some(32));
    let mut outer = Program::new();
    fs(&mut outer, false, 0x38);
    outer.code.extend([0x48, 0x89, 0xc7]);
    rip(&mut outer.code, first, &[0x48, 0x8d, 0x35], INIT);
    rip(&mut outer.code, first, &[0xff, 0x15], 0xf00);
    outer.expect(0);
    fs(&mut outer, false, 0x28);
    outer.code.extend([0x48, 0x83, 0xc0, 1]);
    fs(&mut outer, true, 0x28);
    let mut inner = Program::new();
    inner.reg(0, 41);
    fs(&mut inner, true, 0x28);
    inner.reg(0, u64::MAX);
    let main = image_with_callbacks(
        &code,
        &[
            (first, finish_callback(outer)),
            (INIT, finish_callback(inner)),
        ],
    );
    check(&main, calls + 1, 2, 0);
}

#[test]
fn native_once_bad_target_unreturned_initializer_and_recursive_wait_never_claim_completion() {
    for kind in 0..3 {
        let mut p = Program::new();
        p.set(8, 0);
        p.ptr(0, 8);
        fs(&mut p, true, 0x30);
        p.ptr(7, 8);
        if kind == 0 {
            p.reg(6, u64::MAX);
        } else {
            p.path(6, INIT);
        }
        p.call(0);
        let (code, calls) = p.finish(None);
        let callback = if kind == 2 {
            let mut init = Program::new();
            fs(&mut init, false, 0x30);
            init.code.extend([0x48, 0x89, 0xc7]);
            rip(&mut init.code, INIT, &[0x48, 0x8d, 0x35], INIT);
            rip(&mut init.code, INIT, &[0xff, 0x15], 0xf00);
            finish_callback(init)
        } else {
            vec![0x0f, 0x0b]
        };
        let main = image_with_callbacks(&code, &[(INIT, callback)]);
        let (status, fields, text) = probe(&main);
        assert_eq!(status, 2, "{text}");
        assert_eq!(
            fields["code"],
            if kind == 1 {
                "unsupported-instruction"
            } else {
                "runtime-service"
            }
        );
        assert_eq!(
            fields["service_stops"],
            (calls + u64::from(kind == 2)).to_string()
        );
        assert_eq!(
            fields["phase"],
            if kind == 0 {
                "main-entry"
            } else {
                "initializer"
            }
        );
        assert_eq!(fields["initializer_calls"], "0");
        assert!(!fields.contains_key("value"));
    }
}
