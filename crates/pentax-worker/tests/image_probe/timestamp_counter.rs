// SPDX-License-Identifier: GPL-2.0-only
//! Original instruction samples bracketed by the real kernel TSC services.

use super::*;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn flags(p: &mut Program) {
    p.reg(0, 1);
    p.reg(2, 2);
    p.code.extend([0x48, 0x39, 0xd0]);
}
fn check_flags(p: &mut Program) {
    p.code
        .extend([0x0f, 0x92, 0xc0, 0x0f, 0x9a, 0xc2, 0x0f, 0x94, 0xc1]);
    p.code
        .extend([0x0f, 0xb6, 0xc0, 0x0f, 0xb6, 0xd2, 0x0f, 0xb6, 0xc9]);
    p.code
        .extend([0xc1, 0xe2, 2, 0xc1, 0xe1, 6, 0x09, 0xd0, 0x09, 0xc8]);
    p.expect(5);
}
fn at_least(p: &mut Program, later: u8, earlier: u8) {
    p.load(0, later);
    p.load(10, earlier);
    p.code
        .extend([0x4c, 0x39, 0xd0, 0x0f, 0x93, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.expect(1);
}

#[test]
fn native_rdtsc_rdtscp_share_kernel_epoch_frequency_and_preserve_guest_state() {
    use pentax_kernel::Service;
    let imports: Vec<_> = [Service::ReadTsc, Service::TscFrequency]
        .into_iter()
        .map(|service| {
            Service::registrations()
                .iter()
                .find(|entry| entry.service == service)
                .unwrap()
                .nid
        })
        .collect();
    let vector = [
        0x1122_3344_5566_7788,
        0x7ff0_0000_0000_0001,
        0x99aa_bbcc_ddee_ff00,
        0x8000_0000_7fc0_1234,
    ];
    let mut p = Program::new();
    for (index, value) in vector.into_iter().enumerate() {
        set_word(&mut p, 96 + index as u8 * 8, value);
    }
    p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 96]);
    set_word(&mut p, 80, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 80]);
    p.call(1);
    p.save(8);
    p.code
        .extend([0x48, 0x85, 0xc0, 0x0f, 0x95, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.expect(1); // Positive invariant-TSC frequency before either instruction.
    p.call(0);
    p.save(16);
    for serial in [false, true] {
        flags(&mut p);
        p.reg(0, u64::MAX);
        p.reg(2, u64::MAX);
        p.reg(1, 0xa5a5_9876_1234_5678);
        if serial {
            p.code.extend([0x0f, 0x01, 0xf9]);
        } else {
            p.code.extend([0x0f, 0x31]);
        }
        p.save(24);
        p.stack(0x89, 2, 32);
        p.stack(0x89, 1, 40);
        check_flags(&mut p);
        p.word(40, if serial { 0 } else { 0xa5a5_9876_1234_5678 });
        for offset in [24, 32] {
            p.load(0, offset);
            p.code.extend([0x48, 0xc1, 0xe8, 32]);
            p.expect(0); // Both full64-bit outputs were zero-extended.
        }
        p.load(0, 24);
        p.load(10, 32);
        p.code.extend([0x49, 0xc1, 0xe2, 32, 0x4c, 0x09, 0xd0]);
        let counter = if serial { 64 } else { 48 };
        let before = if serial { 56 } else { 16 };
        let after = if serial { 72 } else { 56 };
        p.save(counter);
        at_least(&mut p, counter, before);
        p.call(0);
        p.save(after);
        at_least(&mut p, after, counter); // Exact epoch/domain, not process-relative ticks.
    }
    p.call(1);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 80]);
    p.word(80, 0x1f84);
    p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 96]);
    for (index, value) in vector.into_iter().enumerate() {
        p.word(96 + index as u8 * 8, value);
    }
    let (code, calls) = p.finish(None);
    let original = image(&code, &imports);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(calls, 5);
    assert_eq!(fields["integer_ops"], "2", "{text}");
    assert_eq!(fields["integer_bytes"], "0", "{text}");
    assert_eq!(fields["vector_ops"], "4", "{text}");
    assert_eq!(fields["vector_bytes"], "72", "{text}");
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}
