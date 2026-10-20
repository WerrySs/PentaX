// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_process_time_counter_preserves_frequency_and_elapsed_domain() {
    use pentax_kernel::Service;
    let services = [
        Service::ProcessTimeCounter,
        Service::ProcessTimeCounterFrequency,
        Service::ReadTsc,
        Service::TscFrequency,
        Service::ProcessTime,
    ];
    let registrations = Service::registrations();
    let imports: Vec<_> = services
        .iter()
        .map(|service| {
            registrations
                .iter()
                .find(|entry| entry.service == *service)
                .unwrap()
                .nid
        })
        .collect();
    let mut p = Program::new();
    p.call(1);
    p.save(8);
    p.code
        .extend([0x48, 0x85, 0xc0, 0x0f, 0x95, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.expect(1); // Positive tick frequency.
    p.call(3);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(0);
    p.save(16);
    p.call(4);
    p.save(24);
    p.call(0);
    p.save(32);
    p.load(10, 16);
    p.code
        .extend([0x4c, 0x39, 0xd0, 0x0f, 0x93, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.expect(1); // Process ticks never move backwards on this host.
    // Earlier process microseconds <= later process ticks converted to us.
    p.load(0, 32);
    p.reg(1, 1_000_000);
    p.code.extend([0x48, 0xf7, 0xe1]); // unsigned RDX:RAX = ticks * 1e6
    p.load(1, 8);
    p.code.extend([0x48, 0xf7, 0xf1]); // unsigned quotient by frequency
    p.load(10, 24);
    p.code
        .extend([0x4c, 0x39, 0xd0, 0x0f, 0x93, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.expect(1);
    p.call(2);
    p.save(40);
    p.call(2);
    p.load(10, 40);
    p.code
        .extend([0x4c, 0x39, 0xd0, 0x0f, 0x93, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.expect(1);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&image(&code, &imports));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert!(!fields.contains_key("public_function"));
}
