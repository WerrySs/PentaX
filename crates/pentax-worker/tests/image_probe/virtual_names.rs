// SPDX-License-Identifier: GPL-2.0-only
use super::*;
const PAGE: u64 = 0x4000;

fn address(p: &mut Program, offset: u32) {
    p.load(7, 8);
    p.code.extend([0x48, 0x81, 0xc7]);
    p.code.extend(offset.to_le_bytes());
}
fn relative(p: &mut Program, slot: u8, offset: u32) {
    address(p, offset);
    p.load(0, slot);
    p.code.extend([0x48, 0x39, 0xf8]);
    checked(&mut p.code, &mut p.branches);
}
fn query(p: &mut Program, offset: u32, start: u32, end: u32) {
    address(p, offset);
    p.ptr(6, 24);
    p.ptr(2, 32);
    p.ptr(1, 40);
    p.call(3);
    p.expect(0);
    relative(p, 24, start);
    relative(p, 32, end);
    p.word(40, 3);
}

#[test]
fn native_virtual_names_change_semantic_queries_without_touching_backing() {
    let mut p = Program::new();
    p.set(8, 0);
    p.set(40, 0);
    p.ptr(7, 8);
    p.reg(6, 2 * PAGE);
    p.reg(2, 3);
    p.reg(1, 0);
    p.path(8, NAME);
    p.call(0);
    p.expect(0);
    address(&mut p, 0);
    p.code.extend([0x48, 0xc7, 0x07, 42, 0, 0, 0]);
    address(&mut p, 17);
    p.reg(6, 40);
    p.path(2, NAME + 32);
    p.call(1);
    p.expect(0);
    query(&mut p, 20, 17, 57);
    p.reg(7, u64::MAX);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(1);
    p.expect(0x8002_000e);
    p.reg(7, u64::MAX);
    p.reg(6, 1);
    p.path(2, NAME + 64);
    p.call(1);
    p.expect(0x8002_003f);
    address(&mut p, 27);
    p.reg(6, 10);
    p.call(2);
    p.expect(0);
    query(&mut p, 30, 27, 37);
    query(&mut p, 20, 17, 27);
    address(&mut p, 0);
    p.code.extend([0x48, 0x8b, 7]);
    p.expect(42);
    address(&mut p, 0);
    p.reg(6, 2 * PAGE);
    p.call(4);
    p.expect(0);
    p.ptr(7, 8); // Fixed remap reuses the address, with a fresh backing lifetime.
    p.reg(6, 2 * PAGE);
    p.reg(2, 3);
    p.reg(1, 0x10);
    p.call(5);
    p.expect(0);
    query(&mut p, 30, 0, (2 * PAGE) as u32);
    let (code, calls) = p.finish(None);
    let services = [
        pentax_kernel::Service::MapNamedFlexible,
        pentax_kernel::Service::SetVirtualRangeName,
        pentax_kernel::Service::ClearVirtualRangeName,
        pentax_kernel::Service::QueryProtection,
        pentax_kernel::Service::Munmap,
        pentax_kernel::Service::MapFlexible,
    ];
    let nids: Vec<_> = services
        .into_iter()
        .map(|service| {
            pentax_kernel::Service::registrations()
                .iter()
                .find(|registration| registration.service == service)
                .unwrap()
                .nid
        })
        .collect();
    let original = image(&code, &nids);
    let mut bytes = original.bytes.clone();
    put(&mut bytes, NAME as usize, b"initial\0");
    put(&mut bytes, NAME as usize + 32, b"blue\0");
    put(&mut bytes, NAME as usize + 64, &[0x61; 32]);
    bytes[NAME as usize + 96] = 0;
    let (status, fields, text) = probe(&create_input(bytes));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
