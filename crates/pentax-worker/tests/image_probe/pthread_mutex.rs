// SPDX-License-Identifier: GPL-2.0-only
//! Original guest-side checks over real mutex calls and full result registers.

use super::*;

fn check(main: &Input, calls: u64) {
    let (status, fields, text) = probe_arguments(&["--image".into(), main.path.clone().into()]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

fn prefix_word(p: &mut Program, slot: u8, offset: u32, before: u32, set: u8) {
    p.load(11, slot);
    p.code.extend([0x41, 0x8b, 0x83]); // MOV EAX,[R11+offset].
    p.code.extend(offset.to_le_bytes());
    p.expect(u64::from(before));
    if set != 0 {
        p.code.extend([0x41, 0x83, 0x8b]); // OR DWORD PTR [R11+offset],imm8.
        p.code.extend(offset.to_le_bytes());
        p.code.push(set);
        p.code.extend([0x41, 0x8b, 0x83]);
        p.code.extend(offset.to_le_bytes());
        p.expect(u64::from(before | u32::from(set)));
    }
}

#[test]
fn native_mutex_recursive_attributes_copy_and_balanced_release_use_real_guest_slots() {
    // attr-init, settype, init, attr-destroy, lock, trylock, unlock, destroy.
    let mut p = Program::new();
    p.set(0, 0);
    p.set(8, 0);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    prefix_word(&mut p, 0, 60, 0, 0x42);
    p.ptr(7, 0);
    p.reg(6, 2);
    p.call(1);
    p.expect(0);
    p.ptr(7, 8);
    p.ptr(6, 0);
    p.reg(2, 0);
    p.call(2);
    p.expect(0);
    prefix_word(&mut p, 8, 252, 0, 0x24);
    p.ptr(7, 0);
    p.reg(6, 1);
    p.call(1);
    p.expect(0);
    p.ptr(7, 0);
    p.call(3);
    p.expect(0);
    p.word(0, 0);
    for slot in [4, 5, 4] {
        p.ptr(7, 8);
        p.call(slot);
        p.expect(0);
    }
    prefix_word(&mut p, 8, 252, 0x24, 0);
    for _ in 0..2 {
        p.ptr(7, 8);
        p.call(7);
        p.expect(0x8002_0010);
        p.ptr(7, 8);
        p.call(6);
        p.expect(0);
    }
    p.ptr(7, 8);
    p.call(7);
    p.expect(0x8002_0010);
    p.ptr(7, 8);
    p.call(6);
    p.expect(0);
    p.ptr(7, 8);
    p.call(7);
    p.expect(0);
    p.word(8, 0);
    let (code, calls) = p.finish(None);
    let main = image(
        &code,
        &[
            "F8bUHwAG284",
            "iMp8QpE+XO4",
            "cmo1RIYva9o",
            "smWEktiyyG0",
            "9UK1vLZQft4",
            "upoVrzMHFeE",
            "tn3VlD0hG60",
            "2Of0f+3mhhE",
        ],
    );
    check(&main, calls);
}

#[test]
fn native_mutex_protocol_metadata_uses_kernel_and_posix_setters_before_real_locking() {
    let mut p = Program::new();
    p.set(0, 0);
    p.set(8, 0);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    for (service, protocol) in [(1, 1), (2, 2)] {
        p.ptr(7, 0);
        p.reg(6, 0xdead_beef_0000_0000 | protocol);
        p.call(service);
        p.expect(0);
    }
    p.ptr(7, 8);
    p.ptr(6, 0);
    p.reg(2, 0);
    p.call(3);
    p.expect(0);
    p.ptr(7, 0);
    p.reg(6, 0);
    p.call(1);
    p.expect(0);
    p.ptr(7, 0);
    p.call(4);
    p.expect(0);
    p.word(0, 0);
    for service in [5, 6, 7] {
        p.ptr(7, 8);
        p.call(service);
        p.expect(0);
    }
    p.word(8, 0);
    let (code, calls) = p.finish(None);
    let main = posix::scoped_image(
        &code,
        &[
            ("F8bUHwAG284", false),
            ("1FGvU0i9saQ", false),
            ("5txKfcMUAok", true),
            ("cmo1RIYva9o", false),
            ("smWEktiyyG0", false),
            ("9UK1vLZQft4", false),
            ("tn3VlD0hG60", false),
            ("2Of0f+3mhhE", false),
        ],
        1,
    );
    check(&main, calls);
}

#[test]
fn native_mutex_posix_errors_are_positive_preserve_errno_and_use_exact_library_scopes() {
    // errno, kernel-lock, POSIX-lock alias, trylock, unlock, destroy,
    // absolute timedlock (libkernel only), two-argument POSIX init.
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.errno(0);
    p.set_errno(37);
    p.set(8, 0);
    p.set(32, 0);
    p.set(40, 0);
    p.ptr(7, 8);
    p.reg(6, 0);
    p.reg(2, u64::MAX);
    p.call(7);
    p.expect(0);
    p.ptr(7, 8);
    p.call(1);
    p.expect(0);
    p.ptr(7, 8);
    p.call(2);
    p.expect(11);
    p.errno(37);
    p.ptr(7, 8);
    p.call(3);
    p.expect(16);
    p.errno(37);
    p.ptr(7, 8);
    p.ptr(6, 32);
    p.call(6);
    p.expect(60);
    p.errno(37);
    p.ptr(7, 8);
    p.reg(6, 0);
    p.call(6);
    p.expect(22);
    p.errno(37);
    p.ptr(7, 8);
    p.call(5);
    p.expect(16);
    p.errno(37);
    p.ptr(7, 8);
    p.call(4);
    p.expect(0);
    p.ptr(7, 8);
    p.call(4);
    p.expect(1);
    p.errno(37);
    p.ptr(7, 8);
    p.call(5);
    p.expect(0);
    p.word(8, 0);
    p.errno(37);
    let (code, calls) = p.finish(None);
    let main = posix::scoped_image(
        &code,
        &[
            ("9BcDykPmo1I", false),
            ("9UK1vLZQft4", false),
            ("7H0iTOciTLo", false),
            ("K-jXhbt2gn4", true),
            ("2Z+PpY6CaJg", true),
            ("ltCfaGr2JGE", true),
            ("Io9+nTKXZtA", false),
            ("ttHNfU+qDBU", true),
        ],
        1,
    );
    check(&main, calls);
}

#[test]
fn native_mutex_zero_and_adaptive_initializers_keep_deadlock_busy_and_relative_timeout_results() {
    let mut p = Program::new();
    for initializer in [0, 1] {
        p.set(8, initializer);
        p.ptr(7, 8);
        p.call(1);
        p.expect(0x8002_0001);
        p.ptr(7, 8);
        p.call(0);
        p.expect(0);
        p.ptr(7, 8);
        p.call(0);
        p.expect(0x8002_000b);
        p.ptr(7, 8);
        p.call(2);
        p.expect(0x8002_0010);
        p.ptr(7, 8);
        p.reg(6, 0x100_0000_0000);
        p.call(3);
        p.expect(0x8002_003c);
        p.ptr(7, 8);
        p.call(1);
        p.expect(0);
        p.ptr(7, 8);
        p.call(5);
        p.expect(1);
        p.ptr(7, 8);
        p.call(4);
        p.expect(0);
        p.word(8, 0);
    }
    let (code, calls) = p.finish(None);
    let main = image(
        &code,
        &[
            "9UK1vLZQft4",
            "tn3VlD0hG60",
            "upoVrzMHFeE",
            "IafI2PxcPnQ",
            "2Of0f+3mhhE",
            "2Z+PpY6CaJg",
        ],
    );
    check(&main, calls);
}

#[test]
fn native_mutex_invalid_protocol_and_missing_backing_stop_without_a_fabricated_result() {
    for protocol in [false, true] {
        let mut p = Program::new();
        if protocol {
            p.set(8, 0);
            p.ptr(7, 8);
            p.call(0);
            p.expect(0);
            p.ptr(7, 8);
            p.reg(6, 3);
            p.call(1);
        } else {
            p.reg(7, u64::MAX);
            p.call(2);
        }
        let (code, calls) = p.finish(None);
        let main = image(&code, &["F8bUHwAG284", "1FGvU0i9saQ", "9UK1vLZQft4"]);
        let (status, fields, text) = probe_arguments(&["--image".into(), main.path.clone().into()]);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service");
        assert_eq!(fields["service_stops"], calls.to_string());
        assert!(!fields.contains_key("value"));
    }
}
